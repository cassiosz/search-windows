use super::*;
use crate::sleep::*;

/// A row of loaded tabs `a`, `b`, `c`... in order; the first (the blank the
/// window opened with) takes the first address. Returns the ids.
fn row(n: usize) -> (Tabs, Vec<u64>) {
    let mut t = Tabs::new();
    let first = t.selected_id();
    t.get_mut(first).unwrap().url = Some("https://a.test/".into());
    let mut ids = vec![first];
    for i in 1..n {
        let id = t.open_in_front(&format!("https://{}.test/", (b'a' + i as u8) as char), None);
        ids.push(id);
    }
    t.select(ids[0]);
    (t, ids)
}

fn order(t: &Tabs) -> Vec<u64> {
    t.iter().map(|t| t.id).collect()
}

#[test]
fn ids_are_unique_and_never_reused() {
    let (mut t, ids) = row(3);
    t.close(ids[2]);
    let new = t.open_in_front("https://x.test/", None);
    assert!(!ids.contains(&new));
    let mut sorted = ids.clone();
    sorted.push(new);
    assert!(sorted.windows(2).all(|w| w[0] < w[1]));
}

#[test]
fn a_window_opens_on_one_blank_tab() {
    let t = Tabs::new();
    assert_eq!(t.len(), 1);
    assert!(t.selected().is_blank());
}

// Browser.newTab: repeated Ctrl+T must not leave a row of empty tabs.
#[test]
fn new_tab_goes_to_the_end_and_reuses_an_empty_one() {
    let (mut t, ids) = row(3);
    let n = t.new_tab();
    assert_eq!(order(&t), [ids[0], ids[1], ids[2], n]);
    assert_eq!(t.selected_id(), n);
    t.select(ids[0]);
    assert_eq!(t.new_tab(), n);
    assert_eq!(t.len(), 4);
    assert_eq!(t.selected_id(), n);
}

#[test]
fn reused_empty_tab_moves_to_where_a_new_one_goes() {
    let (mut t, ids) = row(2);
    let n = t.new_tab();
    t.options.new_tab_place = NewTabPlace::Top;
    t.pin(ids[0]);
    // Top: first after the pins, not before them.
    assert_eq!(t.new_tab(), n);
    assert_eq!(order(&t), [ids[0], n, ids[1]]);
}

// Browser.placeForFront / placeForNew: Top is after the pins.
#[test]
fn top_places_a_new_tab_after_the_pins() {
    let (mut t, ids) = row(3);
    t.pin(ids[1]);
    t.options.new_tab_place = NewTabPlace::Top;
    let n = t.open_in_front("https://n.test/", None);
    assert_eq!(order(&t), [ids[1], n, ids[0], ids[2]]);
}

// Browser.placeForNew: beside the tab you are on, never among the pins.
#[test]
fn after_current_never_lands_among_pins() {
    let (mut t, ids) = row(3);
    t.pin(ids[0]);
    t.pin(ids[1]);
    t.select(ids[0]);
    t.options.new_tab_place = NewTabPlace::AfterCurrent;
    let at = t.place_for_new();
    assert_eq!(at, 2);
    t.select(ids[2]);
    assert_eq!(t.place_for_new(), 3);
}

// Browser.placeForLink: after the opener, and after links it already opened,
// so a row of them reads in click order.
#[test]
fn background_links_go_after_the_opener_in_click_order() {
    let (mut t, ids) = row(3);
    let a = t.open_in_background("https://1.test/", Some(ids[0]));
    let b = t.open_in_background("https://2.test/", Some(ids[0]));
    let c = t.open_in_background("https://3.test/", Some(ids[0]));
    assert_eq!(order(&t), [ids[0], a, b, c, ids[1], ids[2]]);
    // The tab in front stays in front.
    assert_eq!(t.selected_id(), ids[0]);
    assert_eq!(t.get(a).unwrap().opener, Some(ids[0]));
}

#[test]
fn background_run_ends_when_you_go_to_another_tab() {
    let (mut t, ids) = row(3);
    let a = t.open_in_background("https://1.test/", Some(ids[0]));
    t.select(ids[1]);
    t.select(ids[0]);
    let b = t.open_in_background("https://2.test/", Some(ids[0]));
    // A new run: straight after the page, ahead of the earlier link.
    assert_eq!(order(&t), [ids[0], b, a, ids[1], ids[2]]);
}

#[test]
fn background_runs_of_two_openers_do_not_mix() {
    let (mut t, ids) = row(3);
    let a = t.open_in_background("https://1.test/", Some(ids[0]));
    let b = t.open_in_background("https://2.test/", Some(ids[1]));
    assert_eq!(order(&t), [ids[0], a, ids[1], b, ids[2]]);
}

#[test]
fn a_link_from_a_pin_lands_after_the_pins() {
    let (mut t, ids) = row(3);
    t.pin(ids[0]);
    t.pin(ids[1]);
    let a = t.open_in_background("https://1.test/", Some(ids[0]));
    assert_eq!(t.index_of(a), Some(2));
}

#[test]
fn background_link_with_no_opener_uses_the_selected_tab() {
    let (mut t, ids) = row(2);
    t.select(ids[1]);
    let a = t.open_in_background("https://1.test/", None);
    assert_eq!(order(&t), [ids[0], ids[1], a]);
}

// Browser.close: the neighbour on the right, or the last if there is none.
#[test]
fn closing_the_selected_tab_selects_its_right_neighbour() {
    let (mut t, ids) = row(4);
    t.select(ids[1]);
    assert_eq!(t.close(ids[1]), Some(ids[2]));
    assert_eq!(t.selected_id(), ids[2]);
}

#[test]
fn closing_the_last_selects_the_one_before() {
    let (mut t, ids) = row(3);
    t.select(ids[2]);
    assert_eq!(t.close(ids[2]), Some(ids[1]));
}

#[test]
fn closing_a_background_tab_keeps_the_selection() {
    let (mut t, ids) = row(3);
    t.select(ids[2]);
    assert_eq!(t.close(ids[0]), None);
    assert_eq!(t.selected_id(), ids[2]);
    assert_eq!(t.len(), 2);
}

// Browser.close: with closeGoesBack, the tab viewed before.
#[test]
fn close_goes_back_when_asked() {
    let (mut t, ids) = row(4);
    t.options.close_goes_back = true;
    t.select(ids[3]);
    t.select(ids[1]);
    assert_eq!(t.close(ids[1]), Some(ids[3]));
    // Not asked: the neighbour.
    let (mut t, ids) = row(4);
    t.select(ids[3]);
    t.select(ids[1]);
    assert_eq!(t.close(ids[1]), Some(ids[2]));
}

// Browser.close: a page a link opened goes back to the page it came from
// while that is still the tab you were on before it.
#[test]
fn closing_a_link_goes_back_to_its_opener() {
    let (mut t, ids) = row(3);
    let l = t.open_in_front("https://l.test/", Some(ids[0]));
    t.select(ids[0]);
    t.select(l);
    assert_eq!(t.close(l), Some(ids[0]));
}

#[test]
fn closing_a_link_does_not_go_back_when_the_opener_was_not_the_last_tab() {
    let (mut t, ids) = row(3);
    let l = t.open_in_front("https://l.test/", Some(ids[0]));
    t.select(ids[1]);
    t.select(l);
    // Last left was ids[1], not the opener: the neighbour rule (here the
    // last tab, so the one before it).
    assert_eq!(t.close(l), Some(ids[2]));
}

// Browser.close: the last tab closed leaves a blank one behind; closing the
// blank one closes the window.
#[test]
fn closing_the_only_tab_leaves_a_blank_one() {
    let mut t = Tabs::new();
    let id = t.selected_id();
    t.get_mut(id).unwrap().url = Some("https://a.test/".into());
    assert!(!t.closes_window(id));
    let new = t.close(id).unwrap();
    assert_ne!(new, id);
    assert_eq!(t.len(), 1);
    assert!(t.selected().is_blank());
    assert!(t.closes_window(new));
    assert_eq!(t.close(new), None);
    assert_eq!(t.len(), 1);
}

// Browser.close, onlyPins: a new tab takes the place of the last loose tab.
#[test]
fn new_tab_after_last_leaves_a_new_tab_beside_pins() {
    let (mut t, ids) = row(2);
    t.pin(ids[0]);
    t.options.new_tab_after_last = true;
    t.select(ids[1]);
    let n = t.close(ids[1]).unwrap();
    assert_eq!(order(&t), [ids[0], n]);
    assert!(t.selected().is_blank());
    // Off, the neighbour is picked as before: the pin.
    let (mut t, ids) = row(2);
    t.pin(ids[0]);
    t.select(ids[1]);
    assert_eq!(t.close(ids[1]), Some(ids[0]));
}

// Browser.close: a pin is put down, not closed; you land on the tab left last.
#[test]
fn closing_a_pin_puts_it_down() {
    let (mut t, ids) = row(3);
    t.pin(ids[0]);
    t.set_now(10);
    t.select(ids[2]);
    t.set_now(20);
    t.select(ids[1]);
    t.set_now(30);
    t.select(ids[0]);
    assert_eq!(t.close(ids[0]), Some(ids[1]));
    assert_eq!(t.len(), 3);
    let pin = t.get(ids[0]).unwrap();
    assert!(pin.asleep && pin.pinned);
    // Not offered to reopen: it never closed.
    assert!(!t.can_reopen());
}

// Browser.leave: ordinary tabs first, not another pin.
#[test]
fn putting_down_a_pin_prefers_ordinary_tabs() {
    let (mut t, ids) = row(3);
    t.pin(ids[0]);
    t.pin(ids[1]);
    t.set_now(5);
    t.select(ids[2]);
    t.set_now(50);
    t.select(ids[1]); // the pin was left most recently, the loose tab earlier
    t.set_now(60);
    t.select(ids[0]);
    assert_eq!(t.close(ids[0]), Some(ids[2]));
}

#[test]
fn putting_down_a_pin_with_nothing_else_opens_a_new_tab() {
    let mut t = Tabs::new();
    let id = t.selected_id();
    t.get_mut(id).unwrap().url = Some("https://a.test/".into());
    t.pin(id);
    let n = t.close(id).unwrap();
    assert_ne!(n, id);
    assert!(t.selected().is_blank());
    assert_eq!(order(&t), [id, n]);
}

// Browser.reopen / remember: the last closed tab, at its place, in front.
#[test]
fn reopen_brings_back_the_last_closed_at_its_place() {
    let (mut t, ids) = row(4);
    t.get_mut(ids[1]).unwrap().title = "Bee".into();
    t.close(ids[1]);
    assert_eq!(order(&t), [ids[0], ids[2], ids[3]]);
    let back = t.reopen_closed().unwrap();
    assert_eq!(t.index_of(back), Some(1));
    assert_eq!(t.selected_id(), back);
    let tab = t.get(back).unwrap();
    assert_eq!(tab.url.as_deref(), Some("https://b.test/"));
    assert_eq!(tab.title, "Bee");
    assert!(!tab.asleep);
}

#[test]
fn reopen_goes_back_newest_first_and_runs_out() {
    let (mut t, ids) = row(4);
    t.close(ids[1]);
    t.close(ids[2]);
    let first = t.reopen_closed().unwrap();
    assert_eq!(t.get(first).unwrap().url.as_deref(), Some("https://c.test/"));
    let second = t.reopen_closed().unwrap();
    assert_eq!(t.get(second).unwrap().url.as_deref(), Some("https://b.test/"));
    assert_eq!(t.reopen_closed(), None);
}

// Browser.remember: a blank tab has nothing to come back to.
#[test]
fn a_blank_tab_is_not_remembered() {
    let (mut t, ids) = row(2);
    let n = t.new_tab();
    t.close(n);
    assert!(!t.can_reopen());
    t.close(ids[0]);
    assert!(t.can_reopen());
}

// Browser.remember: twelve steps back.
#[test]
fn only_twelve_closed_tabs_are_kept() {
    let (mut t, _) = row(1);
    for i in 0..15 {
        let id = t.open_in_front(&format!("https://{i}.test/"), None);
        t.close(id);
    }
    let mut n = 0;
    let mut last = String::new();
    while let Some(id) = t.reopen_closed() {
        n += 1;
        last = t.get(id).unwrap().url.clone().unwrap();
    }
    assert_eq!(n, 12);
    assert_eq!(last, "https://3.test/");
}

#[test]
fn reopen_never_lands_among_pins() {
    let (mut t, ids) = row(3);
    t.close(ids[1]);
    t.pin(ids[0]);
    t.pin(ids[2]);
    let back = t.reopen_closed().unwrap();
    assert_eq!(t.index_of(back), Some(2));
}

#[test]
fn reopen_keeps_the_name_and_mute() {
    let (mut t, ids) = row(2);
    t.get_mut(ids[1]).unwrap().name = Some("Mail".into());
    t.get_mut(ids[1]).unwrap().muted = true;
    t.close(ids[1]);
    let b = t.reopen_closed().unwrap();
    assert_eq!(t.get(b).unwrap().name.as_deref(), Some("Mail"));
    assert!(t.get(b).unwrap().muted);
}

// App.swift: Ctrl+1..8 by place, 9 is the last.
#[test]
fn ctrl_number_picks_a_place_and_nine_is_the_last() {
    let (mut t, ids) = row(3);
    assert_eq!(t.select_number(2), Some(ids[1]));
    assert_eq!(t.selected_id(), ids[1]);
    assert_eq!(t.select_number(9), Some(ids[2]));
    assert_eq!(t.select_number(5), None);
    assert_eq!(t.selected_id(), ids[2]);
    assert_eq!(t.select_number(0), None);
    assert_eq!(t.select_number(10), None);
    let (mut t, ids) = row(12);
    assert_eq!(t.select_number(8), Some(ids[7]));
    assert_eq!(t.select_number(9), Some(ids[11]));
}

// Browser.step: round to the first again, either way.
#[test]
fn next_and_previous_wrap_round() {
    let (mut t, ids) = row(3);
    assert_eq!(t.select_next(), ids[1]);
    assert_eq!(t.select_next(), ids[2]);
    assert_eq!(t.select_next(), ids[0]);
    assert_eq!(t.select_previous(), ids[2]);
    assert_eq!(t.select_previous(), ids[1]);
}

#[test]
fn next_with_one_tab_stays_put() {
    let mut t = Tabs::new();
    let id = t.selected_id();
    assert_eq!(t.select_next(), id);
    assert_eq!(t.select_previous(), id);
}

#[test]
fn selecting_an_unknown_tab_does_nothing() {
    let (mut t, ids) = row(2);
    assert!(!t.select(999));
    assert_eq!(t.selected_id(), ids[0]);
}

#[test]
fn selecting_wakes_and_stamps_the_one_left() {
    let (mut t, ids) = row(2);
    t.set_now(1000);
    t.mark_asleep(ids[1]);
    assert!(t.get(ids[1]).unwrap().asleep);
    t.select(ids[1]);
    assert!(!t.get(ids[1]).unwrap().asleep);
    assert_eq!(t.get(ids[0]).unwrap().last_active_ms, 1000);
}

// Browser.pin: pins at the head, in the order they were pinned.
#[test]
fn pins_are_kept_first_in_the_order_pinned() {
    let (mut t, ids) = row(4);
    t.pin(ids[2]);
    t.pin(ids[0]);
    assert_eq!(order(&t), [ids[2], ids[0], ids[1], ids[3]]);
    assert_eq!(t.pinned_count(), 2);
    let p = t.get(ids[2]).unwrap();
    assert_eq!(p.pin_letter.as_deref(), Some("C"));
    assert_eq!(p.home.as_deref(), Some("https://c.test/"));
    // Pinning again changes nothing.
    t.pin(ids[2]);
    assert_eq!(order(&t), [ids[2], ids[0], ids[1], ids[3]]);
}

// Browser.unpin: back to the head of the loose tabs.
#[test]
fn unpin_goes_to_the_head_of_the_loose_tabs() {
    let (mut t, ids) = row(4);
    t.pin(ids[2]);
    t.pin(ids[3]);
    t.unpin(ids[2]);
    assert_eq!(order(&t), [ids[3], ids[2], ids[0], ids[1]]);
    let u = t.get(ids[2]).unwrap();
    assert!(!u.pinned && u.pin_letter.is_none() && u.home.is_none());
}

#[test]
fn pin_letter_comes_from_the_site() {
    assert_eq!(monogram(Some("https://www.github.com/x")), "G");
    assert_eq!(monogram(Some("http://user@example.org:8080/")), "E");
    assert_eq!(monogram(None), "\u{2022}");
    assert_eq!(monogram(Some("https://1password.com")), "1");
}

#[test]
fn move_tab_keeps_pins_first() {
    let (mut t, ids) = row(4);
    t.pin(ids[0]);
    t.pin(ids[1]);
    // A loose tab can't go among the pins.
    assert!(t.move_tab(3, 0));
    assert_eq!(order(&t), [ids[0], ids[1], ids[3], ids[2]]);
    // A pin can't leave them.
    assert!(t.move_tab(0, 3));
    assert_eq!(order(&t), [ids[1], ids[0], ids[3], ids[2]]);
    // Among its own kind it moves freely.
    assert!(t.move_tab(2, 3));
    assert_eq!(order(&t), [ids[1], ids[0], ids[2], ids[3]]);
}

#[test]
fn move_tab_ignores_nonsense_and_noops() {
    let (mut t, ids) = row(3);
    assert!(!t.move_tab(7, 0));
    assert!(!t.move_tab(1, 1));
    assert!(t.move_tab(0, 99));
    assert_eq!(order(&t), [ids[1], ids[2], ids[0]]);
}

// ---- sleep (Sleep.swift) ----

const MIN: u64 = 60 * 1000;

/// Three tabs a (selected), b, c, all left at time 0.
fn idle() -> (Tabs, Vec<u64>) {
    let (t, ids) = row(3);
    (t, ids)
}

#[test]
fn paces_are_two_hours_half_an_hour_ten_minutes() {
    assert_eq!(SleepPace::Later.wait_ms(), 120 * MIN);
    assert_eq!(SleepPace::Normal.wait_ms(), 30 * MIN);
    assert_eq!(SleepPace::Sooner.wait_ms(), 10 * MIN);
    assert_eq!(SleepPace::default(), SleepPace::Normal);
}

#[test]
fn a_tab_away_long_enough_sleeps_but_not_before() {
    let (t, ids) = idle();
    let wait = SleepPace::Normal.wait_ms();
    assert_eq!(t.due_to_sleep(wait - 1, wait), Vec::<u64>::new());
    // Never the one in front.
    assert_eq!(t.due_to_sleep(wait, wait), [ids[1], ids[2]]);
}

#[test]
fn the_one_left_longest_sleeps_first() {
    let (mut t, ids) = idle();
    t.get_mut(ids[1]).unwrap().last_active_ms = 500;
    t.get_mut(ids[2]).unwrap().last_active_ms = 100;
    assert_eq!(t.due_to_sleep(10 * MIN + 500, 10 * MIN), [ids[2], ids[1]]);
}

#[test]
fn memory_pressure_shrinks_the_wait() {
    let (t, ids) = idle();
    assert_eq!(t.due_to_sleep(5 * MIN, WAIT_ON_PRESSURE_WARNING_MS), [ids[1], ids[2]]);
    // Critical: nothing to wait for, but the tab on screen still stays.
    assert_eq!(t.due_to_sleep(0, WAIT_ON_PRESSURE_CRITICAL_MS), [ids[1], ids[2]]);
}

#[test]
fn what_keeps_a_tab_awake() {
    let (mut t, ids) = idle();
    let now = 24 * 60 * MIN;
    let wait = SleepPace::Normal.wait_ms();
    let b = ids[1];
    for set in [
        (|t: &mut Tab| t.pinned = true) as fn(&mut Tab),
        |t| t.loading = true,
        |t| t.audible = true,
        |t| t.busy = true,
        |t| t.keep_awake = true,
        |t| t.asleep = true,
        |t| t.url = None,
    ] {
        let before = t.get(b).unwrap().clone();
        set(t.get_mut(b).unwrap());
        assert_eq!(t.due_to_sleep(now, wait), [ids[2]], "{:?}", t.get(b));
        *t.get_mut(b).unwrap() = before;
    }
    assert_eq!(t.due_to_sleep(now, wait), [ids[1], ids[2]]);
}

#[test]
fn awake_reasons_are_named() {
    let (mut t, ids) = idle();
    assert_eq!(t.awake_because(t.get(ids[0]).unwrap(), false), Some("on screen"));
    t.get_mut(ids[1]).unwrap().audible = true;
    assert_eq!(t.awake_because(t.get(ids[1]).unwrap(), false), Some("playing sound"));
    assert_eq!(t.awake_because(t.get(ids[2]).unwrap(), false), None);
}

// Sleep.swift pinKept: a pin stays awake in the window on screen.
#[test]
fn a_pin_never_sleeps_while_the_window_is_open() {
    let (mut t, ids) = idle();
    t.pin(ids[1]);
    assert_eq!(t.due_to_sleep(1000 * MIN, SleepPace::Sooner.wait_ms()), [ids[2]]);
}

// Sleep.swift sleepShut: with the last window closed nothing is on screen:
// everything the rules let go sleeps now; a pin only if its site can't notify.
#[test]
fn with_the_window_shut_everything_not_kept_sleeps_at_once() {
    let (mut t, ids) = idle();
    t.pin(ids[1]);
    t.get_mut(ids[2]).unwrap().audible = true;
    assert_eq!(t.due_when_shut(), [ids[1], ids[0]]);
    t.get_mut(ids[1]).unwrap().notifies = true;
    assert_eq!(t.due_when_shut(), [ids[0]]);
}

// Closing.swift stays(because:): pinned, named, on screen, blank, busy.
#[test]
fn tabs_that_close_themselves_keep_what_you_said_to_keep() {
    let (mut t, ids) = row(6);
    let day = 24 * 60 * MIN;
    t.pin(ids[1]);
    t.get_mut(ids[2]).unwrap().name = Some("Mail".into());
    t.get_mut(ids[3]).unwrap().audible = true;
    // ids[0] is on screen; ids[4], ids[5] are plain.
    assert_eq!(t.due_to_close(day, day), [ids[4], ids[5]]);
    assert_eq!(t.due_to_close(day - 1, day), Vec::<u64>::new());
}

// Sleeping a tab keeps its place and address.
#[test]
fn mark_asleep_keeps_the_address_and_never_sleeps_the_selected() {
    let (mut t, ids) = idle();
    t.mark_asleep(ids[0]);
    assert!(!t.get(ids[0]).unwrap().asleep);
    t.mark_asleep(ids[1]);
    let b = t.get(ids[1]).unwrap();
    assert!(b.asleep && b.url.is_some());
}

// ---- session (Session.swift) ----

fn temp_path(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("searchwin-tabs-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("session.json")
}

#[test]
fn session_round_trips_through_a_file() {
    let (mut t, ids) = row(4);
    t.pin(ids[2]);
    t.get_mut(ids[1]).unwrap().title = "Bee".into();
    t.get_mut(ids[1]).unwrap().name = Some("B".into());
    t.get_mut(ids[3]).unwrap().muted = true;
    t.select(ids[1]);
    let shape = Shape::from_tabs(&t);
    let path = temp_path("round");
    save(&path, &shape).unwrap();
    assert!(!path.with_extension("json.tmp").exists());
    let back = load(&path);
    assert_eq!(back, shape);
    let restored = back.into_tabs(7);
    let urls: Vec<_> = restored.iter().map(|t| t.url.clone().unwrap()).collect();
    assert_eq!(urls, ["https://c.test/", "https://a.test/", "https://b.test/", "https://d.test/"]);
    assert_eq!(restored.selected().title, "Bee");
    assert_eq!(restored.pinned_count(), 1);
    assert!(restored.get(restored.iter().next().unwrap().id).unwrap().pin_letter.is_some());
    assert!(restored.iter().any(|t| t.muted));
}

// Upstream's file: the same keys, so what it writes reads here.
#[test]
fn reads_what_upstream_writes() {
    let json = r#"{"tabs":[
        {"url":"https://a.test/","title":"A","pin":"A","home":"https://a.test/","pinID":"7E1C0F5A-0000-0000-0000-000000000000"},
        {"url":"https://b.test/","title":"B","name":"Bee","groupID":"7E1C0F5A-0000-0000-0000-000000000001","muted":true},
        {"url":"https://c.test/","title":"C"}],
        "active":2,"groups":[{"id":"x"}],"splits":[{"tabs":[1,2],"sizes":[0.5,0.5]}]}"#;
    let path = temp_path("upstream");
    std::fs::write(&path, json).unwrap();
    let shape = load(&path);
    assert_eq!(shape.tabs.len(), 3);
    assert_eq!(shape.tabs[0].pin.as_deref(), Some("A"));
    assert_eq!(shape.tabs[1].name.as_deref(), Some("Bee"));
    assert!(shape.tabs[1].muted);
    assert_eq!(shape.active, 2);
    // Not used yet, but not dropped on the way back out.
    assert!(shape.groups.is_some());
    assert_eq!(shape.splits.len(), 1);
    save(&path, &shape).unwrap();
    assert_eq!(load(&path), shape);
}

// Session.read: no file is an empty session, and so is one that won't decode.
#[test]
fn a_missing_file_is_an_empty_session() {
    let shape = load(&temp_path("missing"));
    assert!(shape.tabs.is_empty());
    let t = shape.into_tabs(0);
    assert_eq!(t.len(), 1);
    assert!(t.selected().is_blank());
}

#[test]
fn a_corrupt_file_is_an_empty_session_and_is_kept_aside() {
    let path = temp_path("corrupt");
    std::fs::write(&path, b"{ not json \xff\xfe").unwrap();
    assert!(load(&path).tabs.is_empty());
    assert!(!path.exists());
    assert!(path.with_extension("json.bad").exists());
    // Wrong shape is corrupt too.
    std::fs::write(&path, br#"{"tabs":"no"}"#).unwrap();
    assert!(load(&path).tabs.is_empty());
    std::fs::write(&path, b"").unwrap();
    assert!(load(&path).tabs.is_empty());
}

#[test]
fn a_file_of_other_versions_keys_still_reads() {
    let path = temp_path("lenient");
    std::fs::write(&path, br#"{"tabs":[{"url":"https://a.test/","future":1}],"active":0,"more":true}"#).unwrap();
    assert_eq!(load(&path).tabs.len(), 1);
}

#[test]
fn save_replaces_the_old_file_whole() {
    let path = temp_path("atomic");
    let (t, _) = row(3);
    save(&path, &Shape::from_tabs(&t)).unwrap();
    let (t, _) = row(1);
    save(&path, &Shape::from_tabs(&t)).unwrap();
    assert_eq!(load(&path).tabs.len(), 1);
    // Saving into a folder that isn't there is an error, not a panic.
    assert!(save(&path.parent().unwrap().join("no/such/session.json"), &Shape::default()).is_err());
}

// Browser.restoreSession: tabs come back asleep, but for the one in front.
#[test]
fn restored_tabs_are_asleep_but_the_one_in_front() {
    let (t, ids) = row(3);
    let mut shape = Shape::from_tabs(&t);
    shape.active = 1;
    let r = shape.into_tabs(0);
    let asleep: Vec<bool> = r.iter().map(|t| t.asleep).collect();
    assert_eq!(asleep, [true, false, true]);
    assert_eq!(r.selected().url.as_deref(), Some("https://b.test/"));
    assert_ne!(ids[1], 0);
}

// Session.Entry.touched: counts across a quit; none starts counting at launch.
#[test]
fn restored_tabs_keep_their_time_or_start_counting_at_launch() {
    let shape = Shape {
        tabs: vec![
            Entry { url: "https://a.test/".into(), touched: Some(42), ..Default::default() },
            Entry { url: "https://b.test/".into(), ..Default::default() },
        ],
        active: 0,
        ..Default::default()
    };
    let r = shape.into_tabs(900);
    let times: Vec<u64> = r.iter().map(|t| t.last_active_ms).collect();
    assert_eq!(times, [42, 900]);
}

// Session.fresh: only pins and named tabs stay, and none is in front.
#[test]
fn a_fresh_start_keeps_pins_and_names_and_opens_a_blank_tab() {
    let (mut t, ids) = row(4);
    t.pin(ids[0]);
    t.get_mut(ids[2]).unwrap().name = Some("Mail".into());
    let fresh = Shape::from_tabs(&t).fresh();
    assert_eq!(fresh.active, -1);
    assert_eq!(fresh.tabs.len(), 2);
    let r = fresh.into_tabs(0);
    assert_eq!(r.len(), 3);
    assert!(r.selected().is_blank());
    assert_eq!(r.iter().last().unwrap().id, r.selected_id());
}

#[test]
fn an_active_place_that_is_not_there_opens_a_blank_tab() {
    let shape = Shape { tabs: vec![Entry { url: "https://a.test/".into(), ..Default::default() }], active: 9, ..Default::default() };
    let r = shape.into_tabs(0);
    assert_eq!(r.len(), 2);
    assert!(r.selected().is_blank());
}

#[test]
fn blank_tabs_are_written_with_an_empty_address() {
    let t = Tabs::new();
    let shape = Shape::from_tabs(&t);
    assert_eq!(shape.tabs[0].url, "");
    let r = shape.into_tabs(0);
    assert!(r.selected().is_blank());
}

#[test]
fn pins_are_first_even_if_the_file_says_otherwise() {
    let shape = Shape {
        tabs: vec![
            Entry { url: "https://a.test/".into(), ..Default::default() },
            Entry { url: "https://b.test/".into(), pin: Some("B".into()), ..Default::default() },
        ],
        active: 0,
        ..Default::default()
    };
    let r = shape.into_tabs(0);
    assert_eq!(r.iter().next().unwrap().url.as_deref(), Some("https://b.test/"));
    // The one in front is still the one that was.
    assert_eq!(r.selected().url.as_deref(), Some("https://a.test/"));
}

#[test]
fn optional_keys_are_left_out_of_the_file() {
    let json = serde_json::to_string(&Shape {
        tabs: vec![Entry { url: "u".into(), title: "t".into(), ..Default::default() }],
        ..Default::default()
    })
    .unwrap();
    assert_eq!(json, r#"{"tabs":[{"url":"u","title":"t"}],"active":0}"#);
}

// A session with groups and splits must still have them after being loaded
// into a row and saved from it (the row is what the app holds and writes).
#[test]
fn groups_and_splits_survive_load_then_save() {
    let json = r#"{"tabs":[{"url":"https://a.test/","title":"A"},{"url":"https://b.test/","title":"B"}],
        "active":0,"groups":{"g1":{"name":"Work","tabs":[0,1]}},"splits":[{"tabs":[0,1],"sizes":[0.5,0.5]}]}"#;
    let path = temp_path("groups");
    std::fs::write(&path, json).unwrap();
    let row = load(&path).into_tabs(5);
    let again = Shape::from_tabs(&row);
    assert_eq!(again.groups.as_ref().unwrap()["g1"]["name"], "Work");
    assert_eq!(again.splits.len(), 1);
    assert_eq!(again.splits[0]["sizes"][1], 0.5);
    save(&path, &again).unwrap();
    let back = load(&path);
    assert_eq!(back.groups, again.groups);
    assert_eq!(back.splits, again.splits);
}

// Restoring must not spend id 1 on a blank tab that is dropped at once.
#[test]
fn restored_ids_start_at_one() {
    let shape = Shape {
        tabs: vec![Entry { url: "https://a.test/".into(), ..Default::default() }],
        active: 0,
        ..Default::default()
    };
    let row = shape.into_tabs(1);
    assert_eq!(row.selected_id(), 1);
}
