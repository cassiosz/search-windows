//! Split View in the window; crates/split is the model and the geometry.
//! Off until Settings › Tabs › Split View is on, as upstream: until then
//! no split is read, none is made, and the page stands alone in the card.
//!
//! Keys, with it on: Ctrl+Alt+N splits the page in front (a new tab on its
//! right, the field ready), Ctrl+Alt+Left/Right move the keyboard between
//! the pages, Ctrl+W closes the page that has it and the others take its
//! room. Alt+click on a tab puts it beside the page in front; a tab's menu
//! has Split With Current Page and Separate Split Tabs.
//!
//! The tab in front is always the split's focused page, so back, forward,
//! reload, the field and every other page command act on that one.

use searchwin_split::{Drag, Layout, Metrics, Point, Rect, Splits};
use searchwin_tabs::Tabs;
use windows::Win32::Foundation::RECT;

use crate::{State, scale, state};

pub fn on() -> bool {
    crate::internal::prefs(|p| p.split_view)
}

/// PaneStage.swift's pixels, at the window's DPI.
pub fn metrics(dpi: i32) -> Metrics {
    Metrics { gap: scale(7, dpi), narrowest: scale(250, dpi), snap: scale(15, dpi), unsnap: scale(20, dpi) }
}

pub fn rect(r: RECT) -> Rect {
    Rect::new(r.left, r.top, r.right - r.left, r.bottom - r.top)
}

pub fn win(r: Rect) -> RECT {
    RECT { left: r.x, top: r.y, right: r.right(), bottom: r.bottom() }
}

/// A row's splits as its session file has them.
pub fn from_row(tabs: &Tabs) -> Splits {
    let ids: Vec<u64> = tabs.iter().map(|t| t.id).collect();
    let mut splits = searchwin_split::from_session(&tabs.splits, &ids);
    // A pin is never one of the pages beside another.
    splits.list.retain(|s| s.tabs.iter().all(|t| tabs.get(*t).is_some_and(|t| !t.pinned)));
    splits
}

/// The split in front laid out in the card, a divider being dragged with
/// its live shares; None when the tab in front stands alone.
pub fn layout_of(s: &State, card: RECT) -> Option<Layout> {
    let front = s.tabs.selected_id();
    let split = s.splits.split_of(front)?;
    let live;
    let split = match &s.drag {
        Some(drag) => {
            live = drag.apply(split);
            &live
        }
        None => split,
    };
    Some(searchwin_split::layout(rect(card), split, Some(front), &metrics(s.dpi)))
}

/// Where each page of the split in front goes. Side by side, each is drawn
/// a hair inside its place, so the one with the keys can be outlined.
pub fn panes(s: &State, card: RECT) -> Vec<(u64, RECT)> {
    let Some(layout) = layout_of(s, card) else { return Vec::new() };
    let inset = if layout.paired { scale(2, s.dpi) } else { 0 };
    layout
        .panes
        .iter()
        .map(|(id, r)| {
            let r = win(*r);
            (*id, RECT { left: r.left + inset, top: r.top + inset, right: r.right - inset, bottom: r.bottom - inset })
        })
        .collect()
}

/// Its later pages sit right after the first, in order: the session keeps a
/// split only so (searchwin_split::to_session), and the row reads that way.
pub fn gather(tabs: &mut Tabs, splits: &Splits) {
    for split in &splits.list {
        for pair in split.tabs.windows(2) {
            let (Some(before), Some(from)) = (tabs.index_of(pair[0]), tabs.index_of(pair[1])) else { continue };
            let to = if from > before { before + 1 } else { before };
            tabs.move_tab(from, to);
        }
    }
}

/// Settings turned it on or off. Off separates every split: the tabs stay.
pub fn turned(on: bool) {
    state(|s| {
        if on {
            s.splits = from_row(&s.tabs);
        } else {
            s.splits = Splits::new();
            s.tabs.splits.clear();
            s.drag = None;
        }
        s.session_owed = true;
    });
    crate::show_selected();
    crate::layout();
    crate::repaint_all();
}

/// Ctrl+Alt+N: a new tab beside the one in front, the field ready for where
/// it goes (Browser.startSplit). Into the split in front when it has room.
pub fn split_current() -> bool {
    if !on() {
        return false;
    }
    let made = state(|s| {
        let front = s.tabs.selected_id();
        if s.tabs.selected().pinned || s.tabs.selected().is_blank() {
            return false;
        }
        s.tabs.set_now(searchwin_history::now_ms());
        let new = s.tabs.new_tab();
        let made = match s.splits.split_of(front) {
            Some(split) if !split.is_full() => s.splits.add(new, front, false),
            Some(_) => false,
            None => s.splits.split_current(front, new),
        };
        if made {
            gather(&mut s.tabs, &s.splits);
            s.private.remove(&new);
            if let Some(profile) = s.private.get(&front).cloned() {
                s.private.insert(new, profile);
            }
        } else {
            s.tabs.select(front);
        }
        made
    });
    crate::show_selected();
    made.is_some()
}

/// Ctrl+Alt+Left/Right: the keyboard to the page on that side.
pub fn focus_step(left: bool) -> bool {
    if !on() {
        return false;
    }
    let moved = state(|s| {
        let front = s.tabs.selected_id();
        let to = s.splits.focus_step(front, left)?;
        (to != front).then(|| s.tabs.select(to))
    })
    .flatten();
    if moved == Some(true) {
        crate::show_selected();
    }
    true
}

/// Alt+click on a tab, or Split With Current Page: `id` beside the page in
/// front, in its split when there is room, and the keyboard with it.
pub fn join(id: u64) -> bool {
    let joined = state(|s| {
        let front = s.tabs.selected_id();
        let pinned = |t: u64| s.tabs.get(t).is_none_or(|t| t.pinned);
        if pinned(id) || pinned(front) || !s.splits.can_join(id, front) || !s.splits.add(id, front, false) {
            return false;
        }
        gather(&mut s.tabs, &s.splits);
        s.tabs.set_now(searchwin_history::now_ms());
        s.tabs.select(id);
        s.session_owed = true;
        true
    }) == Some(true);
    if joined {
        crate::show_selected();
    }
    joined
}

/// Separate Split Tabs: every page of the split back to a tab of its own.
pub fn separate(id: u64) {
    if state(|s| s.splits.separate(id).is_some()) == Some(true) {
        state(|s| s.session_owed = true);
        crate::show_selected();
        crate::layout();
        crate::repaint_all();
    }
}

/// The divider under the point, if any (client coordinates).
pub fn divider_at(x: i32, y: i32) -> Option<usize> {
    let card = crate::card_rect(crate::client_rect(), state(|s| s.dpi)?);
    state(|s| {
        let layout = layout_of(s, card)?;
        searchwin_split::hit_test(&layout, Point { x, y }, scale(3, s.dpi))
    })
    .flatten()
}

/// A divider taken hold of.
pub fn begin_drag(index: usize) -> bool {
    let card = crate::card_rect(crate::client_rect(), state(|s| s.dpi).unwrap_or(96));
    state(|s| {
        let split = s.splits.split_of(s.tabs.selected_id())?;
        let length = if split.axis == searchwin_split::Axis::Horizontal { card.right - card.left } else { card.bottom - card.top };
        s.drag = Some(Drag::begin(split, index, length, &metrics(s.dpi))?);
        Some(())
    })
    .flatten()
    .is_some()
}

/// The pointer moved while holding it: x along the card.
pub fn drag_to(x: i32) {
    let card = crate::card_rect(crate::client_rect(), state(|s| s.dpi).unwrap_or(96));
    let moved = state(|s| s.drag.as_mut().is_some_and(|d| d.to(x - card.left))) == Some(true);
    if moved {
        crate::place_pages();
        crate::repaint_card();
    }
}

/// Let go: the shares are kept.
pub fn end_drag() {
    let done = state(|s| {
        let drag = s.drag.take()?;
        let front = s.tabs.selected_id();
        s.splits.split_of_mut(front)?.set_sizes(drag.shares());
        s.session_owed = true;
        Some(())
    })
    .flatten();
    if done.is_some() {
        crate::place_pages();
        crate::repaint_card();
        crate::save_soon();
    }
}

/// A double click on a divider evens the pages out (Browser.swift:4179).
pub fn even_out() {
    state(|s| {
        let front = s.tabs.selected_id();
        s.splits.even_out(front);
        s.session_owed = true;
    });
    crate::place_pages();
    crate::repaint_card();
    crate::save_soon();
}
