use super::*;
use searchwin_bookmarks::Node;
use searchwin_prefs::Prefs;
use searchwin_spaces::{FIRST_SPACE_ID, PERSONAL_ID, Space};

fn items(pairs: &[(&str, &str)]) -> Items {
    pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
}

fn all(_: &str, _: &str) -> bool {
    true
}

const A: &str = "AAAA";
const B: &str = "BBBB";

// ---- the merge ----

#[test]
fn an_edit_here_wins_over_what_was_agreed() {
    let base = pass(&items(&[("k", "1")]), None, &[], A, 100.0, &all);
    let theirs = base.clone();
    let after = pass(&items(&[("k", "2")]), Some(&base), &[theirs], A, 200.0, &all);
    assert_eq!(live(&after)["k"], "2");
}

#[test]
fn a_clock_behind_still_beats_what_it_has_seen() {
    let base = pass(&items(&[("k", "1")]), None, &[], A, 1000.0, &all);
    // This PC's clock now says 10: the edit is still stamped after 1000.
    let after = pass(&items(&[("k", "2")]), Some(&base), std::slice::from_ref(&base), A, 10.0, &all);
    assert!(after["k"].t > 1000.0);
    assert_eq!(live(&after)["k"], "2");
}

#[test]
fn the_latest_stamp_wins_and_every_order_agrees() {
    let one = Map::from([("k".into(), Entry { v: Some("a".into()), t: 5.0, d: A.into() })]);
    let two = Map::from([("k".into(), Entry { v: Some("b".into()), t: 6.0, d: B.into() })]);
    assert_eq!(merge(&[one.clone(), two.clone()]), merge(&[two.clone(), one.clone()]));
    assert_eq!(live(&merge(&[one, two]))["k"], "b");
    // A tie: the larger device, then a tombstone, then the larger text.
    let x = Entry { v: Some("x".into()), t: 1.0, d: A.into() };
    let y = Entry { v: Some("y".into()), t: 1.0, d: B.into() };
    assert!(wins(&y, &x) && !wins(&x, &y));
    let gone = Entry { v: None, t: 1.0, d: A.into() };
    assert!(wins(&gone, &x) && !wins(&x, &gone));
}

#[test]
fn a_deletion_travels_and_a_later_edit_brings_it_back() {
    let base = pass(&items(&[("bm:1", "{}")]), None, &[], A, 100.0, &all);
    let deleted = pass(&Items::new(), Some(&base), &[], A, 200.0, &all);
    assert!(deleted["bm:1"].v.is_none());
    // Another PC edited it before the deletion: the deletion wins.
    let edited = Map::from([("bm:1".into(), Entry { v: Some("{\"x\":1}".into()), t: 150.0, d: B.into() })]);
    assert!(live(&merge(&[deleted.clone(), edited])).is_empty());
    // After: the edit wins.
    let later = Map::from([("bm:1".into(), Entry { v: Some("{\"x\":2}".into()), t: 300.0, d: B.into() })]);
    assert_eq!(live(&merge(&[deleted, later]))["bm:1"], "{\"x\":2}");
}

#[test]
fn old_deletions_are_forgotten() {
    let map = Map::from([("k".into(), Entry { v: None, t: 0.0, d: A.into() })]);
    assert!(prune(map.clone(), TOMBSTONE_LIFE - 1.0).contains_key("k"));
    assert!(prune(map, TOMBSTONE_LIFE + 1.0).is_empty());
}

#[test]
fn a_whole_kind_gone_at_once_comes_back() {
    let five: Vec<(String, String)> = (0..5).map(|i| (format!("bm:{i}"), "{}".to_string())).collect();
    let current: Items = five.into_iter().collect();
    let base = pass(&current, None, &[], A, 100.0, &all);
    let after = pass(&Items::new(), Some(&base), &[], A, 200.0, &all);
    assert_eq!(live(&after).len(), 5, "nothing deleted");
    // Four can be deleted: that is a choice.
    let four: Items = (0..4).map(|i| (format!("bm:{i}"), "{}".to_string())).collect();
    let base = pass(&four, None, &[], A, 100.0, &all);
    assert!(live(&pass(&Items::new(), Some(&base), &[], A, 200.0, &all)).is_empty());
}

#[test]
fn a_pc_joining_takes_what_is_there_and_keeps_its_own() {
    let theirs = pass(&items(&[("set:shield", "false")]), None, &[], B, 100.0, &all);
    let joined = pass(&items(&[("set:shield", "true"), ("bm:mine", "{}")]), None, &[theirs], A, 200.0, &all);
    let live = live(&joined);
    assert_eq!(live["set:shield"], "false");
    assert!(live.contains_key("bm:mine"));
}

#[test]
fn what_this_build_cant_read_is_never_deleted() {
    let base = pass(&items(&[("new:thing", "1")]), None, &[], B, 100.0, &all);
    let after = pass(&Items::new(), Some(&base), &[], A, 200.0, &understood);
    assert_eq!(live(&after)["new:thing"], "1");
}

#[test]
fn files_of_another_kind_are_not_read() {
    let file = File { format: FORMAT, device: B.into(), name: None, scope: "app".into(), written: 1.0, items: Map::new() };
    let data = encode(&file);
    assert!(decode(&data, "app").is_some());
    assert!(decode(&data, "profile-x").is_none());
    assert!(decode(b"{nonsense", "app").is_none());
    let mut other = file.clone();
    other.format = 2;
    assert!(decode(&encode(&other), "app").is_none());
    other = file;
    other.device = "..\\evil".into();
    assert!(decode(&encode(&other), "app").is_none());
}

#[test]
fn an_order_keeps_what_it_did_not_know() {
    let ids = ["a", "b", "c", "d"].into_iter().map(String::from).collect();
    let order = ["c".to_string(), "a".to_string()];
    let fallback = ["b".to_string()];
    assert_eq!(ordered(&ids, &order, &fallback), ["c", "a", "b", "d"]);
}

// ---- the items ----

#[test]
fn settings_round_trip_and_bad_ones_are_refused() {
    let prefs = Prefs { shield: false, ..Prefs::default() };
    let here = settings(&Prefs::default());
    let there = settings(&prefs);
    assert_eq!(settings_to_apply(&there, &here), [("shield".to_string(), serde_json::json!(false))]);
    let mut bad = here.clone();
    bad.insert("set:shield".into(), "\"loud\"".into());
    bad.insert("set:custom_engine".into(), "\"javascript:alert(1)//%s\"".into());
    assert!(settings_to_apply(&bad, &here).is_empty());
    // A custom search address never comes from the folder, however it looks.
    let mut engine = here.clone();
    engine.insert("set:custom_engine".into(), "\"https://find.example/?q=%s\"".into());
    assert!(settings_to_apply(&engine, &here).is_empty());
    assert!(!here.contains_key("set:custom_engine"));
    assert!(!understood("set:window_size", "1"));
}

#[test]
fn a_bookmark_this_build_cant_read_stays_as_it_is_here() {
    let mut mine = Node::site("Mine", "https://mine.example/");
    mine.id = "S1".into();
    let live = items(&[("kids:root", "[\"S1\"]"), ("bm:S1", "{\"newer\":true}")]);
    let tree = apply_bookmarks(&live, std::slice::from_ref(&mine));
    assert_eq!(tree, [mine]);
}

#[test]
fn a_chain_of_folders_stays_shallow() {
    // A folder in a folder, a hundred thousand deep: no stack overflow.
    let n = 100_000;
    let mut live = Items::new();
    for i in 0..n {
        live.insert(format!("bm:F{i}"), "{\"title\":\"f\"}".into());
        live.insert(format!("kids:F{i}"), format!("[\"F{}\"]", i + 1));
    }
    live.insert("kids:root".into(), "[\"F0\"]".into());
    let tree = apply_bookmarks(&live, &[]);
    fn depth(nodes: &[Node]) -> usize {
        nodes.iter().map(|n| 1 + depth(n.children.as_deref().unwrap_or_default())).max().unwrap_or(0)
    }
    assert!(depth(&tree) <= 64);
}

#[test]
fn a_space_this_build_cant_read_is_never_synced_away() {
    let first = Space::first();
    let work = space("W1", "Work");
    let live = items(&[("space:W1", "{\"name\":1}")]);
    let out = apply_spaces(&live, &[&first, &work], PERSONAL_ID, FIRST_SPACE_ID);
    assert!(out.iter().any(|s| s.id == "W1" && s.name == "Work"));
}

#[test]
fn names_lose_direction_overrides() {
    assert_eq!(clean("abc\u{202E}gpj.exe\u{0007}"), "abcgpj.exe");
}

#[test]
fn bookmarks_round_trip() {
    let mut site = Node::site("Rust", "https://rust-lang.org/");
    site.id = "S1".into();
    let mut folder = Node::folder("Work", vec![site]);
    folder.id = "F1".into();
    let mut other = Node::site("Docs", "https://docs.rs/");
    other.id = "S2".into();
    let roots = vec![folder, other];
    assert_eq!(apply_bookmarks(&bookmarks(&roots), &[]), roots);
}

#[test]
fn unsafe_bookmarks_never_land_and_are_never_deleted() {
    let live = items(&[
        ("kids:root", "[\"S1\",\"S2\"]"),
        ("bm:S1", "{\"title\":\"x\",\"url\":\"javascript:alert(1)\"}"),
        ("bm:S2", "{\"title\":\"ok\\u0007\",\"url\":\"https://a.example/\"}"),
    ]);
    let tree = apply_bookmarks(&live, &[]);
    assert_eq!(tree.len(), 1);
    assert_eq!(tree[0].title, "ok");
    assert!(!understood("bm:S1", &live["bm:S1"]));
}

#[test]
fn an_orphan_bookmark_goes_to_the_top() {
    // Its folder was deleted on one PC while it was added on another.
    let live = items(&[("bm:S1", "{\"title\":\"x\",\"url\":\"https://a.example/\"}"), ("kids:F1", "[\"S1\"]")]);
    let tree = apply_bookmarks(&live, &[]);
    assert_eq!(tree.len(), 1);
    assert_eq!(tree[0].id, "S1");
}

fn space(id: &str, name: &str) -> Space {
    Space { id: id.into(), name: name.into(), colour: 0, icon: None, shares_sign_ins: false, downloads: None, profile: None }
}

#[test]
fn spaces_round_trip_and_keep_what_is_this_pcs() {
    let mut work = space("W1", "Work");
    work.downloads = Some("D:\\work".into());
    let first = Space::first();
    let local = [&first, &work];
    let mut live = spaces(&local, &Items::new());
    live.insert("space:W1".into(), "{\"colour\":2,\"name\":\"Trabalho\",\"sharesSignIns\":true}".into());
    let out = apply_spaces(&live, &local, PERSONAL_ID, FIRST_SPACE_ID);
    assert_eq!(out.len(), 2);
    assert_eq!(out[1].name, "Trabalho");
    assert_eq!(out[1].colour, 2);
    assert_eq!(out[1].downloads.as_deref(), Some("D:\\work"));
    assert!(!out[1].shares_sign_ins, "a space's store never changes under it");
}

#[test]
fn a_new_space_arrives_and_the_home_never_goes() {
    let first = Space::first();
    let live = items(&[
        ("space:N1", "{\"colour\":99,\"icon\":\"nope\",\"name\":\"\",\"sharesSignIns\":true}"),
        ("order:spaces", "[\"N1\"]"),
    ]);
    let out = apply_spaces(&live, &[&first], PERSONAL_ID, FIRST_SPACE_ID);
    assert_eq!(out.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), ["N1", FIRST_SPACE_ID]);
    assert_eq!((out[0].colour, out[0].icon.clone(), out[0].name.as_str()), (0, None, "Space"));
    assert!(out[0].shares_sign_ins);
}

// ---- the folder ----

#[test]
fn two_pcs_meet_in_a_folder() {
    let dir = std::env::temp_dir().join(format!("searchwin-sync-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let root = dir.join(INSIDE);
    let state_a = dir.join("a").join("sync-app.json");
    let state_b = dir.join("b").join("sync-app.json");
    std::fs::create_dir_all(state_a.parent().unwrap()).unwrap();
    std::fs::create_dir_all(state_b.parent().unwrap()).unwrap();
    let mine = items(&[("set:shield", "false")]);
    let a = run(&root, "app", &state_a, A, "PC-A", 100.0, |_| mine.clone()).unwrap();
    assert!(a.others.is_empty());
    let b = run(&root, "app", &state_b, B, "PC-B", 200.0, |_| items(&[("set:shield", "true")])).unwrap();
    assert_eq!(b.others, ["PC-A"]);
    assert_eq!(b.live["set:shield"], "false", "joining takes what is there");
    // A conflict copy and a stray file are passed over.
    std::fs::write(root.join("app").join("AAAA-PC.json"), b"{}").unwrap();
    std::fs::write(root.join("app").join("notes.txt"), b"x").unwrap();
    let again = run(&root, "app", &state_a, A, "PC-A", 300.0, |_| mine.clone()).unwrap();
    assert_eq!(again.others, ["PC-B"]);
    leave(&root, A);
    assert!(!root.join("app").join("AAAA.json").exists());
    forget(&dir.join("a"));
    assert!(!state_a.exists());
    let _ = std::fs::remove_dir_all(&dir);
}

// ---- pins ----

fn def(id: &str, home: &str) -> PinDef {
    PinDef { id: id.into(), letter: "X".into(), home: home.into(), title: format!("T{id}"), name: None }
}

#[test]
fn pins_round_trip_with_titles_agreed_once() {
    let local = [def("P1", "https://a.example/"), def("P2", "https://b.example/")];
    let items = pins("S1", &local, &Items::new());
    assert_eq!(apply_pins(&items, "S1", &local), local);
    // A title changed by browsing doesn't travel once one is agreed.
    let mut browsed = local.clone();
    browsed[0].title = "Inbox (3)".into();
    assert_eq!(pins("S1", &browsed, &items)["title:pin:P1"], "\"TP1\"");
    // Another space's pins are not this one's.
    assert!(apply_pins(&items, "S2", &[]).is_empty());
}

#[test]
fn a_pin_from_elsewhere_arrives_and_unsafe_ones_dont() {
    let live = items(&[
        ("pin:S1:P9", "{\"home\":\"https://new.example/\",\"letter\":\"N\u{202E}w\"}"),
        ("pin:S1:P8", "{\"home\":\"javascript:alert(1)\",\"letter\":\"J\"}"),
        ("order:pins:S1", "[\"P8\",\"P9\"]"),
    ]);
    let out = apply_pins(&live, "S1", &[]);
    assert_eq!(out.len(), 1);
    assert_eq!((out[0].id.as_str(), out[0].letter.as_str(), out[0].title.as_str()), ("P9", "Nw", "https://new.example/"));
    assert!(!understood("pin:S1:P8", &live["pin:S1:P8"]));
}

#[test]
fn a_pin_this_build_cant_read_stays_as_it_is_here() {
    let mine = def("P1", "search://settings");
    let live = pins("S1", std::slice::from_ref(&mine), &Items::new());
    assert_eq!(apply_pins(&live, "S1", std::slice::from_ref(&mine)), [mine]);
}

#[test]
fn a_flood_of_pins_is_capped_but_ours_stay() {
    let mut live = Items::new();
    for i in 0..1000 {
        live.insert(format!("pin:S1:N{i}"), "{\"home\":\"https://x.example/\",\"letter\":\"X\"}".into());
    }
    let mine = def("MINE", "https://mine.example/");
    live.extend(pins("S1", std::slice::from_ref(&mine), &Items::new()));
    let out = apply_pins(&live, "S1", std::slice::from_ref(&mine));
    assert_eq!(out.len(), PINS_MAX + 1);
    assert!(out.iter().any(|d| d.id == "MINE"));
}
