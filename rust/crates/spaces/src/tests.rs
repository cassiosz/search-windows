use super::*;
use std::path::Path;

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("searchwin-spaces-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn on() -> Spaces {
    let mut s = Spaces::default();
    s.set_uses_spaces(true);
    s
}

// MARK: defaults — today's app exactly

#[test]
fn default_is_one_profile_one_space_off() {
    // Spaces.swift:7-10, :108; Profiles.swift:16-19.
    let s = Spaces::default();
    assert_eq!(s.profiles(), &[Profile::personal()]);
    assert_eq!(s.all(), &[Space::first()]);
    assert_eq!(s.current_id(), FIRST_SPACE_ID);
    assert_eq!(s.current_profile(), PERSONAL_ID);
    assert!(!s.uses_spaces() && !s.sends_links() && !s.profiles_in_one_window());
    assert!(!s.several_profiles());
    assert_eq!(s.webview_profile_for(FIRST_SPACE_ID), None);
}

#[test]
fn missing_file_is_default() {
    let dir = temp("missing");
    assert_eq!(Spaces::load(&spaces_path(&dir)), Spaces::default());
}

#[test]
fn corrupt_file_is_default_and_moved_aside() {
    let dir = temp("corrupt");
    let path = spaces_path(&dir);
    std::fs::write(&path, b"{not json").unwrap();
    assert_eq!(Spaces::load(&path), Spaces::default());
    assert!(!path.exists());
    assert!(dir.join("spaces.json.bad").exists());
}

// MARK: files

#[test]
fn session_file_names_follow_upstream() {
    // Session.swift:61-63.
    assert_eq!(session_file_name(FIRST_SPACE_ID), "session.json");
    assert_eq!(session_file_name("ABC"), "session-ABC.json");
    assert_eq!(session_path(Path::new("D"), "ABC"), Path::new("D").join("session-ABC.json"));
}

#[test]
fn personal_data_stays_where_it_was() {
    // Profiles.swift:16-19, :104-123: Personal's files don't move.
    let base = Path::new("base");
    assert_eq!(profile_folder(base, PERSONAL_ID), base);
    assert_eq!(profile_folder(base, "X"), base.join("Profiles").join("X"));
    assert_eq!(webview_profile(PERSONAL_ID), None);
    assert_eq!(webview_profile("X").as_deref(), Some("profile-X"));
}

#[test]
fn save_and_load_round_trip() {
    let dir = temp("round");
    let path = spaces_path(&dir);
    let mut s = on();
    let work = s.add_space("Work", Some("terminal"), false);
    s.set_colour(&work, 2);
    s.set_downloads(&work, Some("C:\\dl"));
    let p = s.add_profile("Job").unwrap();
    s.set_sends_links(true);
    s.set_route("notion.so", &work);
    s.save(&path).unwrap();
    assert!(!beside(&path, ".tmp").exists());
    let back = Spaces::load(&path);
    assert_eq!(back, s);
    assert_eq!(back.current_id(), work);
    assert_eq!(back.profile(&p).unwrap().name, "Job");
}

#[test]
fn reads_upstream_style_space_entries() {
    // spaces.json entries as Spaces.swift:20-38 encodes them.
    let dir = temp("upstream");
    let path = spaces_path(&dir);
    std::fs::write(
        &path,
        r#"{"spaces":[{"id":"00000000-0000-0000-0000-000000000001","name":"Home","colour":0},
            {"id":"aaaaaaaa-0000-4000-8000-000000000002","name":"Work","colour":3,"icon":"terminal","sharesSignIns":true}],
            "on":true,"current":"AAAAAAAA-0000-4000-8000-000000000002"}"#,
    )
    .unwrap();
    let s = Spaces::load(&path);
    assert_eq!(s.all().len(), 2);
    let work = &s.all()[1];
    assert_eq!(work.id, "AAAAAAAA-0000-4000-8000-000000000002", "ids uppercased as Swift writes them");
    assert!(work.shares_sign_ins);
    assert_eq!(work.tint(), Some(3));
    assert_eq!(s.current_id(), work.id);
}

// MARK: repair (Spaces.swift:106-127, Profiles.swift:57-61)

#[test]
fn repair_keeps_one_first_space_and_inserts_it_if_lost() {
    let dir = temp("repair");
    let path = spaces_path(&dir);
    std::fs::write(
        &path,
        r#"{"spaces":[{"id":"B","name":"B"},{"id":"00000000-0000-0000-0000-000000000001","name":"Mine","profile":"X"},
            {"id":"00000000-0000-0000-0000-000000000001","name":"Dup"}]}"#,
    )
    .unwrap();
    let s = Spaces::load(&path);
    let ids: Vec<&str> = s.all().iter().map(|s| s.id.as_str()).collect();
    assert_eq!(ids, ["B", FIRST_SPACE_ID], "home kept where it was moved, the duplicate dropped");
    assert_eq!(s.all()[1].name, "Mine");
    assert_eq!(s.all()[1].profile, None, "the first space is always Personal's");

    std::fs::write(&path, r#"{"spaces":[{"id":"B","name":"B"}]}"#).unwrap();
    let s = Spaces::load(&path);
    assert_eq!(s.all()[0].id, FIRST_SPACE_ID);
}

#[test]
fn repair_puts_personal_first_and_fixes_homes() {
    let dir = temp("repair-profiles");
    let path = spaces_path(&dir);
    std::fs::write(
        &path,
        r#"{"profiles":[{"id":"P2","name":"Work","home":"GONE"},{"id":"00000000-0000-0000-0000-0000000000a1","name":"Me","home":"X"},
            {"id":"P3","name":"Empty","home":"NONE"}],
            "spaces":[{"id":"S2","name":"W","profile":"P2"},{"id":"S9","name":"Orphan","profile":"NOPE"}],
            "current":"MISSING","on":true}"#,
    )
    .unwrap();
    let s = Spaces::load(&path);
    assert_eq!(s.profiles()[0].id, PERSONAL_ID);
    assert_eq!(s.profiles()[0].name, "Me", "Personal keeps its name");
    assert_eq!(s.profiles()[0].home, FIRST_SPACE_ID, "and its fixed home");
    assert_eq!(s.home_of("P2"), "S2");
    let p3_home = s.home_of("P3").to_string();
    assert_eq!(s.space(&p3_home).unwrap().profile_id(), "P3", "a profile with no space gets a home");
    assert_eq!(s.profile_of("S9"), PERSONAL_ID, "a space of a lost profile is reachable from Personal");
    assert_eq!(s.current_id(), FIRST_SPACE_ID);
}

#[test]
fn spaces_off_at_load_shows_the_home() {
    // Spaces.swift:541-545: with spaces off a window shows the profile's home.
    let mut s = on();
    s.add_space("Work", None, true);
    s.on = false;
    let dir = temp("off");
    let path = spaces_path(&dir);
    s.save(&path).unwrap();
    assert_eq!(Spaces::load(&path).current_id(), FIRST_SPACE_ID);
}

// MARK: making and changing spaces

#[test]
fn add_space_goes_to_the_end_with_a_free_icon_and_comes_on_screen() {
    // Spaces.swift:463-486.
    let mut s = on();
    let a = s.add_space("  Work ", None, true);
    assert_eq!(s.current_id(), a);
    let space = s.space(&a).unwrap();
    assert_eq!(space.name, "Work");
    assert_eq!(space.icon.as_deref(), Some("briefcase"));
    assert!(space.profile.is_none());
    let b = s.add_space("Two", None, true);
    assert_eq!(s.space(&b).unwrap().icon.as_deref(), Some("building.2"));
    assert_eq!(s.all().last().unwrap().id, b);
    assert_ne!(a, b);
    assert_eq!(a.len(), 36);
    assert_eq!(a, a.to_uppercase());
}

#[test]
fn add_space_with_spaces_off_stays_on_home() {
    let mut s = Spaces::default();
    s.add_space("Work", None, true);
    assert_eq!(s.current_id(), FIRST_SPACE_ID);
}

#[test]
fn rename_icon_colour_downloads() {
    // Spaces.swift:517-539.
    let mut s = on();
    let a = s.add_space("A", None, true);
    assert!(!s.rename_space(&a, "   "));
    assert!(s.rename_space(&a, " B "));
    assert_eq!(s.space(&a).unwrap().name, "B");
    assert!(!s.set_icon(&a, "rocket"));
    assert!(s.set_icon(&a, "leaf"));
    assert_eq!(s.space(&a).unwrap().symbol(), "leaf");
    assert!(!s.set_colour(&a, 6));
    assert!(s.set_colour(&a, 5));
    assert_eq!(s.space(&a).unwrap().tint(), Some(5));
    assert!(s.set_colour(&a, 0));
    assert_eq!(s.space(&a).unwrap().tint(), None);
    assert!(s.set_downloads(&a, Some("D:\\x")));
    assert_eq!(s.downloads_folder(), Some("D:\\x"));
    s.set_uses_spaces(false);
    assert_eq!(s.downloads_folder(), None, "Settings' folder with spaces off (Spaces.swift:345)");
}

#[test]
fn symbol_and_tint_fallbacks() {
    // Spaces.swift:49, :53.
    let mut first = Space::first();
    assert_eq!(first.symbol(), "house");
    first.icon = Some("from-a-newer-version".into());
    assert_eq!(first.symbol(), "house");
    let other = Space { id: "X".into(), colour: 9, ..Space::first() };
    assert_eq!(other.symbol(), "briefcase");
    assert_eq!(other.tint(), None);
    assert_eq!(wash(0, false), None);
    assert_eq!(wash(9, true), None);
    assert_eq!(wash(1, false), Some(0.07));
    assert_eq!(wash(1, true), Some(0.11));
}

#[test]
fn move_space_reorders_within_its_profile_only() {
    // Spaces.swift:488-494: the home can be moved; other profiles' untouched.
    let mut s = on();
    let a = s.add_space("A", None, true);
    let p = s.add_profile("P").unwrap();
    let p_home = s.home_of(&p).to_string();
    let b = s.add_space("B", None, true);
    // Personal: [first, a, b]; P's home sits between a and b in the vec.
    assert!(s.move_space(&b, 0));
    let mine: Vec<&str> = s.of_profile(PERSONAL_ID).map(|s| s.id.as_str()).collect();
    assert_eq!(mine, [b.as_str(), FIRST_SPACE_ID, a.as_str()]);
    assert_eq!(s.all()[2].id, p_home, "the other profile's space keeps its place");
    assert!(!s.move_space(&b, 0), "no move");
    assert!(!s.move_space(&b, 3), "out of range");
    assert!(!s.move_space("nope", 1));
}

#[test]
fn delete_space_rules() {
    // Spaces.swift:548-565.
    let mut s = on();
    assert!(s.delete_space(FIRST_SPACE_ID).is_none(), "the first stays");
    let shared = s.add_space("Shared", None, true);
    let own = s.add_space("Own", None, false);
    assert_eq!(s.current_id(), own);
    s.set_sends_links(true);
    s.set_route("x.com", &own);
    let r = s.delete_space(&own).unwrap();
    assert_eq!(s.current_id(), FIRST_SPACE_ID, "on screen: the home comes");
    assert_eq!(r.spaces, std::slice::from_ref(&own));
    assert_eq!(r.webview_profiles, [format!("space-{own}")], "its own sign-ins are erased");
    assert_eq!(s.erasing(), r.webview_profiles.as_slice());
    assert_eq!(s.routes().len(), 1, "a rule naming it stays, passed over");
    assert!(s.route("https://x.com").is_none());
    let r = s.delete_space(&shared).unwrap();
    assert!(r.webview_profiles.is_empty(), "signed in with the others: nothing of its own");
    assert!(s.delete_space(&shared).is_none());
}

#[test]
fn a_profiles_home_cannot_be_deleted() {
    let mut s = on();
    let p = s.add_profile("P").unwrap();
    let home = s.home_of(&p).to_string();
    assert!(s.delete_space(&home).is_none());
}

#[test]
fn removal_erases_files_but_never_personal_ones() {
    let dir = temp("erase");
    for name in ["session.json", "session-A.json", "history.json"] {
        std::fs::write(dir.join(name), b"{}").unwrap();
    }
    std::fs::create_dir_all(profile_folder(&dir, "P")).unwrap();
    std::fs::write(profile_folder(&dir, "P").join("history.json"), b"{}").unwrap();
    Removal { spaces: vec![FIRST_SPACE_ID.into(), "A".into()], webview_profiles: vec![], profile: Some("P".into()) }
        .erase_files(&dir);
    assert!(dir.join("session.json").exists());
    assert!(!dir.join("session-A.json").exists());
    assert!(!profile_folder(&dir, "P").exists());
    Removal { profile: Some(PERSONAL_ID.into()), ..Removal::default() }.erase_files(&dir);
    assert!(dir.join("history.json").exists(), "Personal's folder is the data folder: never removed");
}

#[test]
fn erasing_list_clears() {
    let mut s = on();
    let a = s.add_space("A", None, false);
    s.delete_space(&a);
    let name = format!("space-{a}");
    assert_eq!(s.erasing(), std::slice::from_ref(&name));
    s.erased(&name);
    assert!(s.erasing().is_empty());
}

// MARK: web view profiles (Spaces.swift:198-211)

#[test]
fn which_store_a_space_uses() {
    let mut s = on();
    let shared = s.add_space("S", None, true);
    let own = s.add_space("O", None, false);
    assert_eq!(s.webview_profile_for(FIRST_SPACE_ID), None);
    assert_eq!(s.webview_profile_for(&shared), None, "Personal's store");
    assert_eq!(s.webview_profile_for(&own), Some(format!("space-{own}")));
    let p = s.add_profile("P").unwrap();
    let home = s.home_of(&p).to_string();
    assert_eq!(s.webview_profile_for(&home), Some(format!("profile-{p}")));
    s.open_profile(&p);
    let p_own = s.add_space("PO", None, false);
    let p_shared = s.add_space("PS", None, true);
    assert_eq!(s.webview_profile_for(&p_own), Some(format!("space-{p_own}")));
    assert_eq!(s.webview_profile_for(&p_shared), Some(format!("profile-{p}")));
    assert_eq!(s.webview_profile_for("unknown"), None);
}

#[test]
fn webview_profile_names_are_valid() {
    // ProfileName: letters, digits, `#@$()+-_~.` and space, at most 64.
    let s = on();
    let name = format!("space-{}", s.new_id());
    assert!(name.len() <= 64);
    assert!(name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'));
}

// MARK: going between spaces

#[test]
fn switching_needs_spaces_on() {
    // Spaces.swift:353.
    let mut s = Spaces::default();
    let a = s.add_space("A", None, true);
    assert!(s.switch_to(&a).is_none());
    assert!(s.select_number(2).is_none());
    assert!(s.neighbour(1).is_none());
    s.set_uses_spaces(true);
    let sw = s.switch_to(&a).unwrap();
    assert_eq!(sw, Switched { from: FIRST_SPACE_ID.into(), to: a.clone(), profile_changed: false });
    assert!(s.switch_to(&a).is_none(), "already there");
    assert!(s.switch_to("nope").is_none());
}

#[test]
fn turning_spaces_off_goes_home_and_keeps_the_others() {
    // Spaces.swift:567-573.
    let mut s = on();
    let a = s.add_space("A", None, true);
    s.set_uses_spaces(false);
    assert_eq!(s.current_id(), FIRST_SPACE_ID);
    assert!(s.space(&a).is_some());
}

#[test]
fn select_number_counts_the_profiles_spaces() {
    // Shortcuts.swift:293-297, Spaces.swift:457-461.
    let mut s = on();
    let a = s.add_space("A", None, true);
    let b = s.add_space("B", None, true);
    assert_eq!(s.select_number(1).unwrap().to, FIRST_SPACE_ID);
    assert_eq!(s.select_number(3).unwrap().to, b);
    assert_eq!(s.select_number(2).unwrap().to, a);
    assert!(s.select_number(4).is_none());
    assert!(s.select_number(0).is_none());
    assert!(s.select_number(10).is_none());
    s.move_space(&b, 0);
    assert_eq!(s.select_number(1).unwrap().to, b, "⌃1–⌃9 follow the order");
}

#[test]
fn next_and_previous_do_not_wrap() {
    // SpaceSwipe.swift:75-96: nothing past either end without other profiles.
    let mut s = on();
    let a = s.add_space("A", None, true);
    assert_eq!(s.current_id(), a);
    assert!(s.next_space().is_none());
    assert_eq!(s.previous_space().unwrap().to, FIRST_SPACE_ID);
    assert!(s.previous_space().is_none());
    assert_eq!(s.next_space().unwrap().to, a);
}

#[test]
fn other_profiles_are_separate_without_one_window() {
    // Profiles.swift:5-15: a window shows its profile's spaces and nothing else.
    let mut s = on();
    let p = s.add_profile("P").unwrap();
    let home = s.home_of(&p).to_string();
    assert_eq!(s.reachable().len(), 1);
    assert!(s.switch_to(&home).is_none());
    assert!(s.next_space().is_none());
    // Choosing the profile is how one gets there.
    let sw = s.open_profile(&p).unwrap();
    assert_eq!(sw.to, home);
    assert!(sw.profile_changed);
    assert_eq!(s.current_profile(), p);
    assert_eq!(s.reachable().iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), [home.as_str()]);
}

#[test]
fn profiles_in_one_window_reach_every_space() {
    // OneWindow.swift:24-26, :256-279.
    let mut s = on();
    let a = s.add_space("A", None, true);
    let p = s.add_profile("P").unwrap();
    let home = s.home_of(&p).to_string();
    s.set_profiles_in_one_window(true);
    assert!(s.merges());
    let ids: Vec<&str> = s.reachable().iter().map(|s| s.id.as_str()).collect();
    assert_eq!(ids, [FIRST_SPACE_ID, a.as_str(), home.as_str()]);
    assert_eq!(s.neighbour(1), Some(home.as_str()), "past the last: the next profile's first");
    let sw = s.select_number(3).unwrap();
    assert!(sw.profile_changed);
    assert_eq!(s.current_profile(), p);
    assert_eq!(s.neighbour(-1), Some(a.as_str()), "before the first: the previous profile's last");
    assert!(s.neighbour(1).is_none());
    assert_eq!(s.previous_space().unwrap().to, a);
    // One profile only: nothing to merge.
    let mut lone = on();
    lone.set_profiles_in_one_window(true);
    assert!(!lone.merges());
}

#[test]
fn a_profile_opens_at_its_last_space() {
    // Profiles.swift:206-208.
    let mut s = on();
    let p = s.add_profile("P").unwrap();
    s.open_profile(&p);
    let x = s.add_space("X", None, true);
    assert_eq!(s.current_id(), x);
    s.open_profile(PERSONAL_ID);
    assert_eq!(s.current_id(), FIRST_SPACE_ID);
    assert_eq!(s.open_profile(&p).unwrap().to, x);
    assert!(s.open_profile("nope").is_none());
    // With spaces off, its home.
    s.open_profile(PERSONAL_ID);
    s.set_uses_spaces(false);
    assert_eq!(s.open_profile(&p).unwrap().to, s.home_of(&p));
}

// MARK: profiles

#[test]
fn add_profile_makes_a_home_that_shares_its_store() {
    // Profiles.swift:127-141.
    let mut s = on();
    assert!(s.add_profile("  ").is_none());
    let p = s.add_profile(" Work ").unwrap();
    let profile = s.profile(&p).unwrap();
    assert_eq!(profile.name, "Work");
    let home = s.space(&profile.home).unwrap();
    assert_eq!(home.name, "Work");
    assert_eq!(home.icon.as_deref(), Some("house"));
    assert!(home.shares_sign_ins);
    assert_eq!(home.profile.as_deref(), Some(p.as_str()));
    assert!(s.several_profiles());
    assert_eq!(s.current_profile(), PERSONAL_ID, "not switched to: the app opens it");
}

#[test]
fn rename_and_move_profiles() {
    let mut s = on();
    let a = s.add_profile("A").unwrap();
    let b = s.add_profile("B").unwrap();
    assert!(!s.rename_profile(&a, " "));
    assert!(s.rename_profile(&a, "Z"));
    assert!(s.rename_profile(PERSONAL_ID, "Me"));
    assert!(!s.move_profile(PERSONAL_ID, 1), "Personal stays first");
    assert!(!s.move_profile(&a, 0));
    assert!(s.move_profile(&b, 1));
    let ids: Vec<&str> = s.profiles().iter().map(|p| p.id.as_str()).collect();
    assert_eq!(ids, [PERSONAL_ID, b.as_str(), a.as_str()]);
}

#[test]
fn delete_profile_takes_everything_and_never_personal() {
    // Profiles.swift:150-185.
    let mut s = on();
    assert!(s.delete_profile(PERSONAL_ID).is_none());
    let p = s.add_profile("P").unwrap();
    let home = s.home_of(&p).to_string();
    s.open_profile(&p);
    let own = s.add_space("Own", None, false);
    let shared = s.add_space("Shared", None, true);
    s.set_sends_links(true);
    s.set_route("x.com", &own);
    let r = s.delete_profile(&p).unwrap();
    assert_eq!(s.current_profile(), PERSONAL_ID, "never left without a window");
    assert_eq!(r.profile.as_deref(), Some(p.as_str()));
    assert_eq!(r.spaces, [home.clone(), own.clone(), shared.clone()]);
    assert_eq!(r.webview_profiles, [format!("space-{own}"), format!("profile-{p}")]);
    assert!(s.profile(&p).is_none());
    assert!(s.space(&home).is_none() && s.space(&own).is_none());
    assert!(!s.several_profiles());
    assert!(s.route("https://x.com").is_none());
    assert_eq!(s.erasing().len(), 2);
    assert!(s.delete_profile(&p).is_none());
}

// MARK: routes (SpaceRoutes.swift)

#[test]
fn site_cleans_what_was_typed() {
    // SpaceRoutes.swift:51-83.
    assert_eq!(site("https://www.Notion.so/x").as_deref(), Some("notion.so"));
    assert_eq!(site("notion.so").as_deref(), Some("notion.so"));
    assert_eq!(site("*.notion.so").as_deref(), Some("notion.so"));
    assert_eq!(site(" Mail.Google.com. ").as_deref(), Some("mail.google.com"));
    assert_eq!(site("www.com").as_deref(), Some("www.com"), "www. kept when nothing would be left");
    assert_eq!(site("localhost").as_deref(), Some("localhost"));
    assert_eq!(site("intranet"), None, "a name with no dot is only localhost");
    assert_eq!(site(""), None);
    assert_eq!(site("*."), None);
    assert_eq!(site("http://[::1]:8080/").as_deref(), Some("::1"));
}

#[test]
fn site_of_only_web_addresses() {
    // SpaceRoutes.swift:63-67.
    assert_eq!(site_of("https://www.github.com/a").as_deref(), Some("github.com"));
    assert_eq!(site_of("http://a.b.example.com:8080/").as_deref(), Some("a.b.example.com"));
    assert_eq!(site_of("file:///C:/x"), None);
    assert_eq!(site_of("search://settings"), None);
    assert_eq!(site_of("not a url"), None);
}

#[test]
fn the_most_specific_site_wins() {
    // SpaceRoutes.swift:87-94.
    let routes = vec![
        Route { site: "google.com".into(), space: "A".into() },
        Route { site: "mail.google.com".into(), space: "B".into() },
    ];
    assert_eq!(find_route("https://mail.google.com/x", &routes).unwrap().space, "B");
    assert_eq!(find_route("https://inbox.mail.google.com", &routes).unwrap().space, "B");
    assert_eq!(find_route("https://docs.google.com", &routes).unwrap().space, "A");
    assert_eq!(find_route("https://google.com", &routes).unwrap().space, "A");
    assert!(find_route("https://notgoogle.com", &routes).is_none(), "a suffix that isn't a domain");
    assert!(find_route("ftp://google.com", &routes).is_none());
}

#[test]
fn routing_is_off_until_both_switches_are_on() {
    // SpaceRoutes.swift:7, :96-101.
    let mut s = Spaces::default();
    let a = s.add_space("A", None, true);
    assert!(s.set_route("notion.so", &a));
    assert!(s.route("https://notion.so").is_none());
    s.set_sends_links(true);
    assert!(s.route("https://notion.so").is_none(), "spaces off");
    s.set_uses_spaces(true);
    assert_eq!(s.route("https://www.notion.so/page").unwrap().id, a);
    assert!(s.route("https://example.com").is_none());
    assert_eq!(s.sends_to("https://notion.so", FIRST_SPACE_ID).unwrap().id, a);
    assert!(s.sends_to("https://notion.so", &a).is_none(), "already in its space");
}

#[test]
fn set_route_replaces_and_validates() {
    // SpaceRoutes.swift:110-122.
    let mut s = on();
    let a = s.add_space("A", None, true);
    assert!(!s.set_route("not a host", &a));
    assert!(!s.set_route("x.com", "missing"));
    assert!(s.set_route("https://www.X.com/", FIRST_SPACE_ID));
    assert!(s.set_route("x.com", &a));
    assert_eq!(s.routes(), &[Route { site: "x.com".into(), space: a.clone() }]);
    assert!(s.remove_route("x.com"));
    assert!(!s.remove_route("x.com"));
}

#[test]
fn labels_name_the_profile_once_there_are_several() {
    // SpaceRoutes.swift:142-147.
    let mut s = on();
    assert_eq!(s.label(FIRST_SPACE_ID).as_deref(), Some("Personal"));
    s.add_profile("Work").unwrap();
    assert_eq!(s.label(FIRST_SPACE_ID).as_deref(), Some("Personal › Personal"));
    assert_eq!(s.label("nope"), None);
}

#[test]
fn shared_pins_show_only_with_spaces() {
    // Prefs.swift:825-833.
    let mut s = Spaces::default();
    assert!(!s.shows_shared_pins());
    s.set_uses_spaces(true);
    assert!(s.shows_shared_pins());
}

// MARK: rows (Spaces.swift:285-293, :403-417)

#[test]
fn rows_read_each_spaces_session_and_write_it_back() {
    let dir = temp("rows");
    let mut first = searchwin_tabs::Tabs::new();
    first.open_in_front("https://a.example/", None);
    searchwin_tabs::save(&session_path(&dir, FIRST_SPACE_ID), &searchwin_tabs::Shape::from_tabs(&first)).unwrap();

    let mut rows = Rows::new();
    let row = rows.row(&dir, FIRST_SPACE_ID, 1);
    assert!(row.iter().any(|t| t.url.as_deref() == Some("https://a.example/")));
    let fresh = rows.row(&dir, "B", 1);
    assert!(fresh.iter().all(|t| t.url.is_none()), "no file yet: a fresh row");
    rows.get_mut("B").unwrap().open_in_front("https://b.example/", None);
    rows.save_all(&dir).unwrap();
    assert!(dir.join("session-B.json").exists());
    let back = searchwin_tabs::load(&dir.join("session-B.json"));
    assert!(back.tabs.iter().any(|e| e.url == "https://b.example/"));
    assert_eq!(rows.spaces().count(), 2);
    assert!(rows.tabs().count() >= 2);
    assert!(rows.remove("B").is_some());
    assert!(rows.get("B").is_none());
    assert!(rows.save(&dir, "B").is_ok(), "nothing in memory: nothing to write");
}

#[test]
fn ids_are_unique() {
    let s = Spaces::default();
    let ids: std::collections::HashSet<String> = (0..1000).map(|_| s.new_id()).collect();
    assert_eq!(ids.len(), 1000);
}
