//! The model: Split.swift's `TabSplit`, and a collection keyed by tab.

use std::collections::HashSet;

/// The most pages one split holds (Split.swift:21).
pub const MOST: usize = 4;

/// Side by side (first on the left) or one above the other (first on top).
/// Split.swift:11-18. Only horizontal is drawn upstream; the file keeps both.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Axis {
    #[default]
    Horizontal,
    Vertical,
}

/// Each page's equal share (Split.swift:136).
pub fn even(count: usize) -> Vec<f64> {
    vec![1.0 / count.max(1) as f64; count]
}

/// The narrowest share a page has, of `count`: a fifth for two, a tenth for
/// three or four (Split.swift:117).
pub fn least(count: usize) -> f64 {
    if count <= 2 { 0.2 } else { 0.1 }
}

/// No page narrower than its least share; shares adding up to 1
/// (Split.swift:120-134).
pub fn clamp(sizes: &[f64]) -> Vec<f64> {
    if sizes.len() == 2 {
        let a = if sizes[0].is_finite() { sizes[0] } else { 0.5 };
        let first = a.clamp(0.2, 0.8);
        return vec![first, 1.0 - first];
    }
    let finite: Vec<f64> = sizes.iter().map(|&s| if s.is_finite() && s > 0.0 { s } else { 1.0 }).collect();
    let total: f64 = finite.iter().sum();
    if total <= 0.0 {
        return even(sizes.len());
    }
    let mut shares: Vec<f64> = finite.iter().map(|s| s / total).collect();
    if shares.len() <= 2 {
        return shares;
    }
    let least = least(shares.len());
    let short: f64 = shares.iter().map(|s| (least - s).max(0.0)).sum();
    if short <= 0.0 {
        return shares;
    }
    let spare: f64 = shares.iter().map(|s| (s - least).max(0.0)).sum();
    for s in &mut shares {
        *s = if *s < least { least } else { *s - (*s - least) / spare * short };
    }
    shares
}

/// Tabs shown side by side (Split.swift `TabSplit`), without its UUID: a tab
/// is in at most one split, so the tab names the split.
#[derive(Clone, Debug, PartialEq)]
pub struct Split {
    /// In the order they are shown; 2..=4.
    pub tabs: Vec<u64>,
    pub axis: Axis,
    /// Each page's share of the stage, same order, adding up to 1.
    pub sizes: Vec<f64>,
    /// The page last focused.
    pub focused: Option<u64>,
}

impl Split {
    /// Even shares unless `sizes` has the right length (Split.swift:32-40).
    pub fn new(tabs: Vec<u64>, axis: Axis, sizes: Option<Vec<f64>>, focused: Option<u64>) -> Split {
        let mut shares = even(tabs.len());
        if let Some(s) = sizes.filter(|s| s.len() == tabs.len()) {
            shares = clamp(&s);
        }
        let focused = focused.filter(|f| tabs.contains(f));
        Split { tabs, axis, sizes: shares, focused }
    }

    /// Two pages with the first's share `fraction` (Split.swift:42).
    pub fn pair(left: u64, right: u64, fraction: f64) -> Split {
        Split::new(vec![left, right], Axis::Horizontal, Some(vec![fraction, 1.0 - fraction]), None)
    }

    /// The first page: the split's place in the row (Split.swift:46).
    pub fn first(&self) -> u64 {
        self.tabs[0]
    }
    pub fn last(&self) -> u64 {
        self.tabs[self.tabs.len() - 1]
    }
    pub fn is_full(&self) -> bool {
        self.tabs.len() >= MOST
    }
    pub fn contains(&self, tab: u64) -> bool {
        self.tabs.contains(&tab)
    }
    pub fn index_of(&self, tab: u64) -> Option<usize> {
        self.tabs.iter().position(|&t| t == tab)
    }

    /// The page beside this one: the next to its right, or for the last the
    /// one to its left (Split.swift:71-74).
    pub fn partner_of(&self, tab: u64) -> Option<u64> {
        let i = self.index_of(tab)?;
        if self.tabs.len() < 2 {
            return None;
        }
        Some(if i + 1 < self.tabs.len() { self.tabs[i + 1] } else { self.tabs[i - 1] })
    }

    /// Whether `tab` stands before `other` (Split.swift:77).
    pub fn is_before(&self, tab: u64, other: u64) -> bool {
        matches!((self.index_of(tab), self.index_of(other)), (Some(a), Some(b)) if a < b)
    }

    /// A tab standing in for another (Split.swift:86-89).
    pub fn replace(&mut self, old: u64, new: u64) {
        for t in &mut self.tabs {
            if *t == old {
                *t = new;
            }
        }
        if self.focused == Some(old) {
            self.focused = Some(new);
        }
    }

    /// A page beside `neighbour` (after it, or before), or at the end; the
    /// pages are made even (Split.swift:96-103). False when full or present.
    pub fn insert(&mut self, tab: u64, beside: Option<u64>, before: bool) -> bool {
        if self.is_full() || self.contains(tab) {
            return false;
        }
        let at = beside
            .and_then(|n| self.index_of(n))
            .map(|i| if before { i } else { i + 1 })
            .unwrap_or(self.tabs.len());
        self.tabs.insert(at, tab);
        self.sizes = even(self.tabs.len());
        true
    }

    /// A page out of it; its share goes to the others in proportion
    /// (Split.swift:110-122). True when two or more are left.
    pub fn remove(&mut self, tab: u64) -> bool {
        let Some(i) = self.index_of(tab) else {
            return self.tabs.len() >= 2;
        };
        self.tabs.remove(i);
        if self.sizes.len() > i {
            self.sizes.remove(i);
        }
        if self.sizes.len() != self.tabs.len() {
            self.sizes = even(self.tabs.len());
        }
        let total: f64 = self.sizes.iter().sum();
        if !self.tabs.is_empty() && total.is_finite() && total > 0.0 {
            let n: Vec<f64> = self.sizes.iter().map(|s| s / total).collect();
            self.sizes = clamp(&n);
        }
        if self.focused == Some(tab) {
            self.focused = None;
        }
        self.tabs.len() >= 2
    }

    /// Swap Pages: the order turned round, shares with it
    /// (Browser.swift:4153-4165).
    pub fn swap(&mut self) {
        self.tabs.reverse();
        self.sizes.reverse();
    }

    /// Move `tab` to place `to` (clamped); shares stay with their places.
    pub fn move_pane(&mut self, tab: u64, to: usize) -> bool {
        let Some(from) = self.index_of(tab) else {
            return false;
        };
        let to = to.min(self.tabs.len() - 1);
        let t = self.tabs.remove(from);
        self.tabs.insert(to, t);
        true
    }

    /// Even Out (Browser.swift:4179).
    pub fn even_out(&mut self) {
        self.sizes = even(self.tabs.len());
    }

    /// Each page's share at once (Browser.setSplitSizes, Browser.swift:3991).
    /// False when wrong length, not finite, or unchanged.
    pub fn set_sizes(&mut self, sizes: &[f64]) -> bool {
        if sizes.len() != self.tabs.len() || !sizes.iter().all(|s| s.is_finite()) {
            return false;
        }
        let s = clamp(sizes);
        if s == self.sizes {
            return false;
        }
        self.sizes = s;
        true
    }

    /// The neighbouring page on that side, none past the ends
    /// (Browser.swift:4033-4038).
    pub fn neighbour(&self, from: u64, left: bool) -> Option<u64> {
        let i = self.index_of(from)?;
        let n = (i as isize + if left { -1 } else { 1 }).clamp(0, self.tabs.len() as isize - 1);
        Some(self.tabs[n as usize])
    }
}

/// What taking a tab out of its split left.
#[derive(Clone, Debug, PartialEq)]
pub enum Removed {
    /// It was in no split.
    None,
    /// Two or more pages remain, sharing the space; `partner` is where the
    /// keys go.
    Shrunk { partner: u64 },
    /// One page was left: the split is gone and that page has the whole area.
    Dissolved { remaining: u64 },
}

/// Every split, a tab in at most one.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Splits {
    pub list: Vec<Split>,
}

impl Splits {
    pub fn new() -> Splits {
        Splits::default()
    }
    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }

    /// The split containing a tab (Browser.split(for:), Browser.swift:136).
    pub fn split_of(&self, tab: u64) -> Option<&Split> {
        self.list.iter().find(|s| s.contains(tab))
    }
    pub fn split_of_mut(&mut self, tab: u64) -> Option<&mut Split> {
        self.list.iter_mut().find(|s| s.contains(tab))
    }

    /// A tab has a place of its own in the row unless it is a later page
    /// (Browser.standsInRow, Browser.swift:144).
    pub fn stands_in_row(&self, tab: u64) -> bool {
        self.split_of(tab).is_none_or(|s| s.first() == tab)
    }

    /// The later pages of every split (Browser.pairedOff, Browser.swift:151).
    pub fn paired_off(&self) -> HashSet<u64> {
        self.list.iter().flat_map(|s| s.tabs.iter().skip(1).copied()).collect()
    }

    /// The tabs on screen with `active` in front (Browser.visibleTabIDs,
    /// Browser.swift:163): its split's pages, or itself.
    pub fn visible(&self, active: u64) -> Vec<u64> {
        self.split_of(active).map_or_else(|| vec![active], |s| s.tabs.clone())
    }

    /// Whether `tab` is shown because its split is the one in front.
    pub fn is_visible(&self, tab: u64, active: u64) -> bool {
        tab == active || self.split_of(active).is_some_and(|s| s.contains(tab))
    }

    /// Whether `tab` can go beside `target`: its split has room, or it is
    /// already in it (Browser.canJoin, Browser.swift:3842-3846).
    pub fn can_join(&self, tab: u64, target: u64) -> bool {
        tab != target && self.split_of(target).is_none_or(|s| s.contains(tab) || !s.is_full())
    }

    /// Split Current Page: `new_tab` goes right of `tab`, focused
    /// (Browser.startSplit, Browser.swift:3924). False when either is
    /// already in a split or they are the same tab.
    pub fn split_current(&mut self, tab: u64, new_tab: u64) -> bool {
        if tab == new_tab || self.split_of(tab).is_some() || self.split_of(new_tab).is_some() {
            return false;
        }
        self.list.push(Split::new(vec![tab, new_tab], Axis::Horizontal, None, Some(new_tab)));
        true
    }

    /// A drop on a page's edge: `dragged` goes beside `target`, left of it
    /// when `on_left`. Into target's split if it has room (pages made even),
    /// otherwise a pair with it; `dragged` leaves any split it was in first
    /// (Browser.pair, Browser.swift:3868-3890). The dragged page is focused.
    pub fn add(&mut self, dragged: u64, target: u64, on_left: bool) -> bool {
        if !self.can_join(dragged, target) {
            return false;
        }
        self.take_out(dragged);
        if let Some(s) = self.split_of_mut(target) {
            s.insert(dragged, Some(target), on_left);
            s.focused = Some(dragged);
        } else {
            let (l, r) = if on_left { (dragged, target) } else { (target, dragged) };
            self.list.push(Split::new(vec![l, r], Axis::Horizontal, None, Some(dragged)));
        }
        true
    }

    /// A pair of exactly these two, each leaving its split first
    /// (`pair(joining: false)`, Browser.swift:3872).
    pub fn pair_exact(&mut self, dragged: u64, target: u64, on_left: bool) -> bool {
        if dragged == target {
            return false;
        }
        self.take_out(dragged);
        self.take_out(target);
        let (l, r) = if on_left { (dragged, target) } else { (target, dragged) };
        self.list.push(Split::new(vec![l, r], Axis::Horizontal, None, Some(dragged)));
        true
    }

    fn take_out(&mut self, tab: u64) -> Removed {
        let Some(i) = self.list.iter().position(|s| s.contains(tab)) else {
            return Removed::None;
        };
        let partner = self.list[i].partner_of(tab).unwrap_or(tab);
        if self.list[i].remove(tab) {
            Removed::Shrunk { partner }
        } else {
            let s = self.list.remove(i);
            Removed::Dissolved { remaining: s.tabs[0] }
        }
    }

    /// One page out of its split (leaveSplit, Browser.swift:3971; also what
    /// closing a focused tab does): the rest share the space and a split of
    /// one dissolves.
    pub fn remove(&mut self, tab: u64) -> Removed {
        self.take_out(tab)
    }

    /// Separate Split Tabs: the whole split dissolved, every tab stays
    /// (detachSplit, Browser.swift:3962). Returns its tabs.
    pub fn separate(&mut self, tab: u64) -> Option<Vec<u64>> {
        let i = self.list.iter().position(|s| s.contains(tab))?;
        Some(self.list.remove(i).tabs)
    }

    /// The tab that takes `from`'s keys with the arrows, remembered as the
    /// split's focus (Browser.focusPane(onLeft:)).
    pub fn focus_step(&mut self, from: u64, left: bool) -> Option<u64> {
        let s = self.split_of_mut(from)?;
        let n = s.neighbour(from, left)?;
        s.focused = Some(n);
        Some(n)
    }
    pub fn focus_next(&mut self, from: u64) -> Option<u64> {
        self.focus_step(from, false)
    }
    pub fn focus_previous(&mut self, from: u64) -> Option<u64> {
        self.focus_step(from, true)
    }

    /// Record which page has the keys (Browser.swift:110).
    pub fn set_focused(&mut self, tab: u64) {
        if let Some(s) = self.split_of_mut(tab) {
            s.focused = Some(tab);
        }
    }

    /// Swap Pages on the split containing `tab`.
    pub fn swap(&mut self, tab: u64) -> bool {
        self.split_of_mut(tab).map(Split::swap).is_some()
    }

    /// Move `tab` to place `to` within its split.
    pub fn move_pane(&mut self, tab: u64, to: usize) -> bool {
        self.split_of_mut(tab).is_some_and(|s| s.move_pane(tab, to))
    }

    pub fn even_out(&mut self, tab: u64) {
        if let Some(s) = self.split_of_mut(tab) {
            s.even_out();
        }
    }

    /// A tab standing in for another (fill, Browser.swift:6218).
    pub fn replace(&mut self, old: u64, new: u64) {
        if let Some(s) = self.split_of_mut(old) {
            s.replace(old, new);
        }
    }
}
