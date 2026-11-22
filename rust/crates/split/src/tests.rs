use super::*;
use serde_json::json;

fn m() -> Metrics {
    Metrics::default()
}

fn split(n: u64) -> Split {
    Split::new((1..=n).collect(), Axis::Horizontal, None, None)
}

fn close(a: &[f64], b: &[f64]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-9)
}

// ---- model: Split.swift

#[test]
fn most_is_four() {
    assert_eq!(MOST, 4); // Split.swift:21
    let mut s = split(4);
    assert!(s.is_full());
    assert!(!s.insert(9, None, false)); // Split.swift:96 guard !isFull
}

#[test]
fn new_sizes_even_or_clamped() {
    assert!(close(&split(3).sizes, &[1.0 / 3.0; 3])); // Split.swift:36
    let s = Split::new(vec![1, 2], Axis::Horizontal, Some(vec![0.95, 0.05]), None);
    assert!(close(&s.sizes, &[0.8, 0.2])); // clamp two, Split.swift:121-123
    // wrong length is ignored (Split.swift:39)
    let s = Split::new(vec![1, 2], Axis::Horizontal, Some(vec![1.0]), None);
    assert!(close(&s.sizes, &[0.5, 0.5]));
    // focused not in tabs is dropped (Split.swift:37)
    assert_eq!(Split::new(vec![1, 2], Axis::Horizontal, None, Some(7)).focused, None);
}

#[test]
fn least_share() {
    assert_eq!(least(2), 0.2); // Split.swift:117
    assert_eq!(least(3), 0.1);
    assert_eq!(least(4), 0.1);
}

#[test]
fn clamp_three_raises_short_pages() {
    let c = clamp(&[0.02, 0.49, 0.49]); // Split.swift:125-134
    assert!(c[0] >= 0.1 - 1e-12);
    assert!((c.iter().sum::<f64>() - 1.0).abs() < 1e-9);
    assert!(close(&clamp(&[f64::NAN, 1.0, 1.0]), &[1.0 / 3.0; 3])); // non-finite -> 1
    assert!(close(&clamp(&[f64::NAN, 0.3]), &[0.5, 0.5]));
}

#[test]
fn partner_and_order() {
    let s = split(3);
    assert_eq!(s.partner_of(1), Some(2)); // Split.swift:71
    assert_eq!(s.partner_of(3), Some(2));
    assert_eq!(s.partner_of(9), None);
    assert!(s.is_before(1, 3));
    assert!(!s.is_before(3, 1));
    assert_eq!((s.first(), s.last()), (1, 3));
}

#[test]
fn insert_beside_makes_even() {
    let mut s = Split::pair(1, 2, 0.7);
    assert!(s.insert(3, Some(1), true)); // before 1
    assert_eq!(s.tabs, vec![3, 1, 2]);
    assert!(close(&s.sizes, &[1.0 / 3.0; 3]));
    assert!(s.insert(4, Some(1), false)); // after 1
    assert_eq!(s.tabs, vec![3, 1, 4, 2]);
    assert!(!s.insert(3, None, false)); // already in it
}

#[test]
fn remove_gives_share_in_proportion() {
    let mut s = Split::new(vec![1, 2, 3], Axis::Horizontal, Some(vec![0.2, 0.3, 0.5]), Some(2));
    assert!(s.remove(2)); // two left, Split.swift:110
    assert!(close(&s.sizes, &[0.2 / 0.7, 0.5 / 0.7]));
    assert_eq!(s.focused, None); // Split.swift:121
    assert!(!s.remove(1)); // one left: no split
}

#[test]
fn swap_reverses_with_sizes() {
    let mut s = Split::new(vec![1, 2, 3], Axis::Horizontal, Some(vec![0.2, 0.3, 0.5]), None);
    s.swap(); // Browser.swift:4157-4158
    assert_eq!(s.tabs, vec![3, 2, 1]);
    assert!(close(&s.sizes, &[0.5, 0.3, 0.2]));
}

#[test]
fn move_pane_keeps_sizes_in_place() {
    let mut s = Split::new(vec![1, 2, 3], Axis::Horizontal, Some(vec![0.2, 0.3, 0.5]), None);
    assert!(s.move_pane(3, 0));
    assert_eq!(s.tabs, vec![3, 1, 2]);
    assert!(close(&s.sizes, &[0.2, 0.3, 0.5]));
    assert!(s.move_pane(3, 99));
    assert_eq!(s.tabs, vec![1, 2, 3]);
    assert!(!s.move_pane(9, 0));
}

#[test]
fn set_sizes_rules() {
    let mut s = split(2);
    assert!(!s.set_sizes(&[0.5, 0.5])); // unchanged
    assert!(!s.set_sizes(&[0.5])); // wrong count, Browser.swift:3993
    assert!(!s.set_sizes(&[f64::NAN, 0.5]));
    assert!(s.set_sizes(&[0.1, 0.9]));
    assert!(close(&s.sizes, &[0.2, 0.8]));
}

#[test]
fn neighbour_stops_at_ends() {
    let s = split(3);
    assert_eq!(s.neighbour(1, true), Some(1)); // Browser.swift:4036 min/max
    assert_eq!(s.neighbour(1, false), Some(2));
    assert_eq!(s.neighbour(3, false), Some(3));
    assert_eq!(s.neighbour(9, false), None);
}

// ---- model: the collection

#[test]
fn split_current_makes_pair_with_new_focused() {
    let mut sp = Splits::new();
    assert!(sp.split_current(1, 2)); // Browser.swift:3924
    let s = sp.split_of(1).unwrap();
    assert_eq!(s.tabs, vec![1, 2]);
    assert_eq!(s.focused, Some(2));
    assert!(!sp.split_current(1, 3)); // already split: ⌥⌘N goes to the right page instead
    assert!(!sp.split_current(3, 3));
}

#[test]
fn add_on_edge_makes_pair_or_joins() {
    let mut sp = Splits::new();
    assert!(sp.add(1, 2, true)); // dropped on target's left edge
    assert_eq!(sp.split_of(2).unwrap().tabs, vec![1, 2]);
    assert!(sp.add(3, 2, false)); // joins, Browser.swift:3879
    assert_eq!(sp.split_of(2).unwrap().tabs, vec![1, 2, 3]);
    assert_eq!(sp.split_of(2).unwrap().focused, Some(3));
    assert!(close(&sp.split_of(2).unwrap().sizes, &[1.0 / 3.0; 3]));
    assert!(sp.add(4, 1, true));
    assert_eq!(sp.split_of(2).unwrap().tabs, vec![4, 1, 2, 3]);
    assert!(!sp.add(5, 1, false)); // full: canJoin
    assert!(!sp.add(1, 1, false));
}

#[test]
fn add_moves_tab_out_of_its_old_split() {
    let mut sp = Splits::new();
    sp.split_current(1, 2);
    sp.split_current(3, 4);
    assert!(sp.add(2, 3, false)); // leaveSplit(dragged), Browser.swift:3873
    assert!(sp.split_of(1).is_none()); // {1,2} dissolved
    assert_eq!(sp.split_of(3).unwrap().tabs, vec![3, 2, 4]);
    // a tab is in at most one split
    for t in 1..=4 {
        assert!(sp.list.iter().filter(|s| s.contains(t)).count() <= 1);
    }
}

#[test]
fn pair_exact_takes_target_out_first() {
    let mut sp = Splits::new();
    sp.add(1, 2, false);
    sp.add(3, 2, false);
    assert!(sp.pair_exact(3, 2, false)); // Browser.swift:3874
    assert_eq!(sp.split_of(2).unwrap().tabs, vec![2, 3]);
    assert!(sp.split_of(1).is_none());
}

#[test]
fn remove_gives_remaining_the_space() {
    let mut sp = Splits::new();
    sp.add(1, 2, true);
    sp.add(3, 2, false);
    assert_eq!(sp.remove(2), Removed::Shrunk { partner: 3 }); // Browser.swift:3973
    assert_eq!(sp.split_of(1).unwrap().tabs, vec![1, 3]);
    assert!(close(&sp.split_of(1).unwrap().sizes, &[0.5, 0.5]));
    assert_eq!(sp.remove(1), Removed::Dissolved { remaining: 3 }); // of two, no split
    assert!(sp.is_empty());
    assert_eq!(sp.remove(3), Removed::None);
}

#[test]
fn separate_keeps_tabs() {
    let mut sp = Splits::new();
    sp.split_current(1, 2);
    assert_eq!(sp.separate(2), Some(vec![1, 2])); // detachSplit, Browser.swift:3962
    assert!(sp.is_empty());
    assert_eq!(sp.separate(2), None);
}

#[test]
fn focus_steps_remember_focus() {
    let mut sp = Splits::new();
    sp.add(1, 2, true);
    sp.add(3, 2, false);
    assert_eq!(sp.focus_next(1), Some(2)); // ⌃⌘→
    assert_eq!(sp.split_of(1).unwrap().focused, Some(2));
    assert_eq!(sp.focus_previous(2), Some(1)); // ⌃⌘←
    assert_eq!(sp.focus_previous(1), Some(1)); // none past the ends
    assert_eq!(sp.focus_next(9), None);
}

#[test]
fn row_and_visibility() {
    let mut sp = Splits::new();
    sp.add(1, 2, true); // [1,2]
    sp.add(3, 2, false); // [1,2,3]
    assert!(sp.stands_in_row(1)); // Browser.swift:144
    assert!(!sp.stands_in_row(2));
    assert!(sp.stands_in_row(9));
    assert_eq!(sp.paired_off(), [2, 3].into_iter().collect());
    assert_eq!(sp.visible(2), vec![1, 2, 3]);
    assert_eq!(sp.visible(9), vec![9]);
    assert!(sp.is_visible(3, 1));
    assert!(!sp.is_visible(9, 1));
    assert!(sp.is_visible(9, 9));
}

#[test]
fn swap_and_move_via_collection() {
    let mut sp = Splits::new();
    sp.split_current(1, 2);
    assert!(sp.swap(1));
    assert_eq!(sp.split_of(1).unwrap().tabs, vec![2, 1]);
    assert!(sp.move_pane(1, 0));
    assert_eq!(sp.split_of(1).unwrap().tabs, vec![1, 2]);
    assert!(!sp.swap(9));
    sp.replace(2, 7); // fill(), Browser.swift:6218
    assert_eq!(sp.split_of(7).unwrap().focused, Some(7));
}

// ---- geometry

fn area(w: i32) -> Rect {
    Rect::new(10, 20, w, 600)
}

/// Panes and gaps cover the area along the axis with nothing between or
/// over: each pane starts where the last divider ended.
fn assert_tiles(l: &Layout, a: Rect, axis: Axis, gap: i32) {
    assert!(l.paired);
    let mut at = if axis == Axis::Horizontal { a.x } else { a.y };
    for (i, (_, r)) in l.panes.iter().enumerate() {
        let (start, len, cross_start, cross_len) =
            if axis == Axis::Horizontal { (r.x, r.w, r.y, r.h) } else { (r.y, r.h, r.x, r.w) };
        assert_eq!(start, at, "pane {i} starts where the last ended");
        assert!(len > 0);
        let (ca, cl) = if axis == Axis::Horizontal { (a.y, a.h) } else { (a.x, a.w) };
        assert_eq!((cross_start, cross_len), (ca, cl));
        at += len;
        if i + 1 < l.panes.len() {
            let d = l.dividers[i];
            let (ds, dl) = if axis == Axis::Horizontal { (d.x, d.w) } else { (d.y, d.h) };
            assert_eq!((ds, dl), (at, gap));
            at += gap;
        }
    }
    assert_eq!(at, if axis == Axis::Horizontal { a.right() } else { a.bottom() });
}

#[test]
fn two_panes_even_side_by_side() {
    let a = area(1007); // room = 1000
    let l = layout(a, &split(2), None, &m());
    assert_eq!(l.panes[0], (1, Rect::new(10, 20, 500, 600))); // PaneStage.swift:411-413
    assert_eq!(l.panes[1], (2, Rect::new(517, 20, 500, 600)));
    assert_eq!(l.dividers, vec![Rect::new(510, 20, 7, 600)]); // PaneStage.swift:417
    assert_tiles(&l, a, Axis::Horizontal, 7);
}

#[test]
fn three_and_four_panes_tile() {
    for n in [3, 4] {
        let a = area(1200);
        let l = layout(a, &split(n), None, &m());
        assert_eq!(l.panes.len(), n as usize);
        assert_eq!(l.dividers.len(), n as usize - 1);
        assert_tiles(&l, a, Axis::Horizontal, 7);
    }
}

#[test]
fn two_pane_left_clamped_to_narrowest() {
    let s = Split::new(vec![1, 2], Axis::Horizontal, Some(vec![0.2, 0.8]), None);
    let l = layout(area(707), &s, None, &m()); // room 700: 0.2*700=140 -> 250
    assert_eq!(l.panes[0].1.w, 250); // PaneStage.swift:489 clampLeft
    assert_eq!(l.panes[1].1.w, 450);
}

#[test]
fn three_pane_shortfall_taken_from_others() {
    let mut s = split(3);
    s.sizes = vec![0.05, 0.45, 0.5]; // stored directly, as a file might
    let a = area(1814); // room 1800
    let l = layout(a, &s, None, &m());
    assert_eq!(l.panes[0].1.w, 250); // PaneStage.swift:511
    assert!(l.panes[1].1.w > 250 && l.panes[2].1.w > 250);
    assert_tiles(&l, a, Axis::Horizontal, 7);
}

#[test]
fn no_room_shows_focused_alone() {
    let s = split(3);
    // fits(3): 3*250 + 2*7 = 764
    assert!(fits(3, 764, &m()));
    assert!(!fits(3, 763, &m()));
    let a = area(700);
    let l = layout(a, &s, Some(2), &m()); // PaneStage.swift:392-399
    assert_eq!(l.panes, vec![(2, a)]);
    assert!(l.dividers.is_empty() && !l.paired);
    assert_eq!(layout(a, &s, None, &m()).panes, vec![(1, a)]);
    assert_eq!(layout(a, &s, Some(99), &m()).panes, vec![(1, a)]);
}

#[test]
fn vertical_axis_stacks() {
    let s = Split::new(vec![1, 2, 3], Axis::Vertical, None, None);
    let a = Rect::new(0, 0, 400, 1000);
    let l = layout(a, &s, None, &m());
    assert_tiles(&l, a, Axis::Vertical, 7);
}

#[test]
fn property_tiling_exact() {
    // LCG: no dependencies.
    let mut seed = 0x2545F4914F6CDD1Du64;
    let mut next = move || {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (seed >> 33) as u32
    };
    for _ in 0..3000 {
        let n = 2 + (next() % 3) as usize;
        let tabs: Vec<u64> = (1..=n as u64).collect();
        let raw: Vec<f64> = (0..n).map(|_| (next() % 1000) as f64 / 100.0 + 0.001).collect();
        let sizes = clamp(&raw);
        let axis = if next() % 4 == 0 { Axis::Vertical } else { Axis::Horizontal };
        let s = Split { tabs, axis, sizes, focused: None };
        let a = Rect::new((next() % 50) as i32, (next() % 50) as i32, 300 + (next() % 3000) as i32, 300 + (next() % 3000) as i32);
        let mm = Metrics { gap: 1 + (next() % 12) as i32, ..m() };
        let l = layout(a, &s, None, &mm);
        let len = if axis == Axis::Horizontal { a.w } else { a.h };
        if fits(n, len, &mm) {
            assert_tiles(&l, a, axis, mm.gap);
        } else {
            assert_eq!(l.panes.len(), 1);
        }
    }
}

#[test]
fn hit_test_finds_divider() {
    let l = layout(area(1214), &split(3), None, &m());
    let d1 = l.dividers[1];
    assert_eq!(hit_test(&l, Point { x: d1.x + 3, y: 100 }, 0), Some(1));
    assert_eq!(hit_test(&l, Point { x: d1.x - 2, y: 100 }, 0), None);
    assert_eq!(hit_test(&l, Point { x: d1.x - 2, y: 100 }, 3), Some(1));
    assert_eq!(hit_test(&l, Point { x: d1.right(), y: 100 }, 0), None); // right edge exclusive
    assert_eq!(hit_test(&l, Point { x: l.panes[0].1.x + 5, y: 100 }, 0), None);
    assert_eq!(hit_test(&l, Point { x: d1.x + 3, y: 5000 }, 0), None);
}

// ---- drag

#[test]
fn drag_two_follows_pointer() {
    let s = Split::pair(1, 2, 0.5);
    let mut d = Drag::begin(&s, 0, 1007, &m()).unwrap(); // room 1000
    // pointer at x=300+3.5: left = 300 -> share .3, not within snap of 500
    assert!(d.to(304));
    assert!(close(d.shares(), &[300.5 / 1000.0, 1.0 - 300.5 / 1000.0]) || (d.shares()[0] - 0.3).abs() < 0.01);
    assert!((d.shares().iter().sum::<f64>() - 1.0).abs() < 1e-12);
}

#[test]
fn drag_respects_narrowest() {
    let s = Split::pair(1, 2, 0.5);
    let mut d = Drag::begin(&s, 0, 1007, &m()).unwrap();
    d.to(-500);
    assert!((d.shares()[0] - 0.25).abs() < 1e-9); // 250/1000, PaneStage.swift:489
    d.to(5000);
    assert!((d.shares()[0] - 0.75).abs() < 1e-9);
}

#[test]
fn drag_snaps_to_even_and_lets_go() {
    let s = Split::pair(1, 2, 0.3);
    let mut d = Drag::begin(&s, 0, 1007, &m()).unwrap();
    d.to(500 + 3); // left ~ 499.5: within 15 of 500
    assert!(d.just_snapped);
    assert!((d.shares()[0] - 0.5).abs() < 1e-9); // PaneStage.swift:546-553
    d.to(500 + 3 + 18); // inside unsnap (20): stays even
    assert!((d.shares()[0] - 0.5).abs() < 1e-9);
    d.to(500 + 3 + 30); // past it: lets go
    assert!(d.shares()[0] > 0.52);
}

#[test]
fn drag_middle_divider_of_four_leaves_others() {
    let s = split(4);
    // room 1200: 300 each
    let mut d = Drag::begin(&s, 1, 1221, &m()).unwrap();
    let before = d.shares().to_vec();
    assert!(d.to(590)); // relative to the area: left page 280 wide, not near even
    let now = d.shares();
    assert_eq!(now[0], before[0]);
    assert_eq!(now[3], before[3]);
    assert!(now[1] < before[1] && now[2] > before[2]);
    assert!((now[1] + now[2] - before[1] - before[2]).abs() < 1e-12);
    // the live shares lay out as a tiling too
    let live = layout(area(1221), &d.apply(&s), None, &m());
    assert_tiles(&live, area(1221), Axis::Horizontal, 7);
}

#[test]
fn drag_by_delta_and_resized() {
    let s = Split::pair(1, 2, 0.4);
    let r = resized(&s, 0, 100, 1007, &m()).unwrap(); // left 400 -> 500
    assert!((r[0] - 0.5).abs() < 1e-9); // lands on the even snap anyway
    let r = resized(&s, 0, -1000, 1007, &m()).unwrap();
    assert!((r[0] - 0.25).abs() < 1e-9);
    assert!(resized(&s, 1, 10, 1007, &m()).is_none()); // no divider after the last
    assert!(Drag::begin(&s, 5, 1007, &m()).is_none());
}

#[test]
fn drag_commit_to_split() {
    let mut s = split(3);
    let mut d = Drag::begin(&s, 0, 1214, &m()).unwrap();
    d.to(100);
    assert!(s.set_sizes(d.shares())); // Browser.setSplitSizes
    assert!((s.sizes.iter().sum::<f64>() - 1.0).abs() < 1e-9);
}

// ---- session shape (Session.swift:51-58, 172-199)

#[test]
fn reads_upstream_shape() {
    let v = vec![json!({"tabs":[1,2,3],"axis":"horizontal","sizes":[0.25,0.25,0.5],"focused":2})];
    let ids = [10, 11, 12, 13];
    let sp = from_session(&v, &ids);
    let s = &sp.list[0];
    assert_eq!(s.tabs, vec![11, 12, 13]);
    assert!(close(&s.sizes, &[0.25, 0.25, 0.5]));
    assert_eq!(s.focused, Some(12));
    assert_eq!(s.axis, Axis::Horizontal);
}

#[test]
fn reads_first_main_shape_and_unknown_axis() {
    let v = vec![
        json!({"left":0,"right":1,"fraction":0.3}), // Session.swift:177-188
        json!({"tabs":[2,3],"axis":"diagonal","sizes":[0.5,0.5]}), // Session.swift:190
    ];
    let sp = from_session(&v, &[1, 2, 3, 4]);
    assert_eq!(sp.list.len(), 2);
    assert!(close(&sp.list[0].sizes, &[0.3, 0.7]));
    assert_eq!(sp.list[1].axis, Axis::Horizontal);
    let v = vec![json!({"tabs":[0,1],"axis":"vertical"})]; // sizes missing -> even
    let sp = from_session(&v, &[1, 2]);
    assert_eq!(sp.list[0].axis, Axis::Vertical);
    assert!(close(&sp.list[0].sizes, &[0.5, 0.5]));
}

#[test]
fn drops_unstandable_splits() {
    let ids = [1, 2, 3, 4, 5, 6];
    let v = vec![
        json!({"tabs":[0],"sizes":[1.0]}),             // one page
        json!({"tabs":[0,9]}),                         // out of range
        json!({"tabs":[0,0]}),                         // duplicate
        json!({"tabs":[0,1,2,3,4]}),                   // five
        json!({"tabs":"x"}),                           // garbage
        json!(7),                                      // garbage
        json!({"tabs":[0,1]}),                         // ok
        json!({"tabs":[1,2]}),                         // tab 1 already taken
        json!({"tabs":[2,3],"focused":0}),             // ok; focused not in it -> None
    ];
    let sp = from_session(&v, &ids);
    assert_eq!(sp.list.len(), 2);
    assert_eq!(sp.list[1].tabs, vec![3, 4]);
    assert_eq!(sp.list[1].focused, None);
}

#[test]
fn round_trips() {
    let ids = [10, 11, 12, 13, 14];
    let v = vec![
        json!({"tabs":[0,1],"axis":"horizontal","sizes":[0.3,0.7],"focused":1}),
        json!({"tabs":[2,3,4],"axis":"vertical","sizes":[0.2,0.3,0.5]}),
    ];
    let sp = from_session(&v, &ids);
    assert_eq!(to_session(&sp, &ids), v);
}

#[test]
fn writes_what_upstream_writes() {
    let mut sp = Splits::new();
    sp.split_current(10, 11);
    let out = to_session(&sp, &[10, 11]);
    // keys: tabs, axis, sizes, focused (Session.swift:197-200)
    assert_eq!(out[0], json!({"tabs":[0,1],"axis":"horizontal","sizes":[0.5,0.5],"focused":1}));
}

#[test]
fn unwritten_pages_and_gaps_drop_pages() {
    let mut sp = Splits::new();
    sp.add(1, 2, true);
    sp.add(3, 2, false); // [1,2,3] even
    // page 2 is not written (empty/private): two left and adjacent, shares made up
    let out = to_session(&sp, &[1, 3]); // Browser.swift:3777-3785
    assert_eq!(out[0]["tabs"], json!([0, 1]));
    // two-page clamp keeps the first share (Split.swift:121-123)
    assert_eq!(out[0]["sizes"], json!([1.0 / 3.0, 1.0 - 1.0 / 3.0]));
    // one written page: a tab again
    assert!(to_session(&sp, &[1]).is_empty());
    // not adjacent in the file: dropped
    assert!(to_session(&sp, &[1, 9, 3]).is_empty());
    // tabs absent from order are skipped for focus
    let mut sp = Splits::new();
    sp.split_current(1, 2);
    sp.set_focused(2);
    let out = to_session(&sp, &[1, 2, 5]);
    assert_eq!(out[0]["focused"], json!(1));
    assert!(to_session(&sp, &[1]).is_empty());
}
