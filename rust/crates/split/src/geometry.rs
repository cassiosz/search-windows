//! Where the panes and dividers go: PaneStage.swift's `layout()`, `widths`,
//! and the divider drag, in integer pixels.

use crate::model::{Axis, Split, clamp, even};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Point {
    pub x: i32,
    pub y: i32,
}

impl Rect {
    pub fn new(x: i32, y: i32, w: i32, h: i32) -> Rect {
        Rect { x, y, w, h }
    }
    pub fn right(&self) -> i32 {
        self.x + self.w
    }
    pub fn bottom(&self) -> i32 {
        self.y + self.h
    }
    pub fn contains(&self, p: Point) -> bool {
        p.x >= self.x && p.x < self.right() && p.y >= self.y && p.y < self.bottom()
    }
    fn inflate(&self, d: i32) -> Rect {
        Rect::new(self.x - d, self.y - d, self.w + 2 * d, self.h + 2 * d)
    }
}

/// The pixel constants (PaneStage.swift:84-94), in physical pixels: scale
/// them by the window's DPI.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Metrics {
    /// Between the pages: room for the divider (`gutter`, 7).
    pub gap: i32,
    /// Narrower than this a page is no use (`narrowest`, 250): the split
    /// shows the focused page alone until there is room again.
    pub narrowest: i32,
    /// How close to even a divider has to come to settle there (`snap`, 15).
    pub snap: i32,
    /// How far past it the pointer goes before it lets go (`unsnap`, 20).
    pub unsnap: i32,
}

impl Default for Metrics {
    fn default() -> Metrics {
        Metrics { gap: 7, narrowest: 250, snap: 15, unsnap: 20 }
    }
}

/// What to draw: the panes on screen with their frames, and the dividers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layout {
    /// Panes shown, in order. With no room for all, only the focused one,
    /// filling the area (PaneStage.shown, PaneStage.swift:392-399).
    pub panes: Vec<(u64, Rect)>,
    /// The divider after pane `i` is `dividers[i]`; empty unless `paired`.
    pub dividers: Vec<Rect>,
    /// All pages are on screen side by side (PaneStage.paired).
    pub paired: bool,
}

/// Room for `count` pages along `length`, none narrower than `narrowest`
/// (PaneStage.fits, PaneStage.swift:376).
pub fn fits(count: usize, length: i32, m: &Metrics) -> bool {
    let n = count as i64;
    length as i64 - (n - 1).max(0) * m.gap as i64 >= n * m.narrowest as i64
}

fn length_of(area: Rect, axis: Axis) -> i32 {
    if axis == Axis::Horizontal { area.w } else { area.h }
}

/// Room for the pages between the dividers (PaneStage.room).
fn room(length: i32, count: usize, m: &Metrics) -> i32 {
    length - (count.saturating_sub(1) as i32) * m.gap
}

/// Each page's length as drawn from its share, none under `narrowest`
/// (PaneStage.widths, PaneStage.swift:497-514), as real numbers.
fn widths_f(room: f64, shares: &[f64], narrowest: f64) -> Vec<f64> {
    if shares.len() == 2 {
        let left = (room * shares[0]).max(narrowest).min(room - narrowest);
        return vec![left, room - left];
    }
    let mut w: Vec<f64> = shares.iter().map(|s| room * s).collect();
    let short: f64 = w.iter().map(|x| (narrowest - x).max(0.0)).sum();
    if short <= 0.0 {
        return w;
    }
    let spare: f64 = w.iter().map(|x| (x - narrowest).max(0.0)).sum();
    if spare <= 0.0 {
        return vec![room / w.len() as f64; w.len()];
    }
    for x in &mut w {
        *x = if *x < narrowest { narrowest } else { *x - (*x - narrowest) / spare * short };
    }
    w
}

/// Integer lengths adding up to exactly `room`: edges are rounded
/// cumulatively, so no pixel is lost or counted twice.
fn widths(room: i32, shares: &[f64], narrowest: i32) -> Vec<i32> {
    let f = widths_f(room as f64, shares, narrowest as f64);
    let total: f64 = f.iter().sum();
    let scale = if total > 0.0 { room as f64 / total } else { 1.0 };
    let (mut out, mut acc, mut prev) = (Vec::with_capacity(f.len()), 0.0, 0);
    for (i, x) in f.iter().enumerate() {
        acc += x * scale;
        let edge = if i + 1 == f.len() { room } else { (acc.round() as i32).clamp(prev, room) };
        out.push(edge - prev);
        prev = edge;
    }
    out
}

/// Place `split`'s panes in `area` with the divider `m.gap` between each
/// two. `focused` is the page with the keys (shown alone when there is no
/// room for all). Panes and gaps tile the area along the axis exactly.
pub fn layout(area: Rect, split: &Split, focused: Option<u64>, m: &Metrics) -> Layout {
    let n = split.tabs.len();
    let len = length_of(area, split.axis);
    if n < 2 || !fits(n, len, m) {
        let one = focused.filter(|f| split.contains(*f)).or(split.tabs.first().copied());
        return Layout { panes: one.into_iter().map(|t| (t, area)).collect(), dividers: vec![], paired: false };
    }
    let w = widths(room(len, n, m), &split.sizes_or_even(), m.narrowest);
    let (mut panes, mut dividers, mut at) = (Vec::with_capacity(n), Vec::with_capacity(n - 1), 0);
    for (i, &tab) in split.tabs.iter().enumerate() {
        let (pane, line) = match split.axis {
            Axis::Horizontal => (
                Rect::new(area.x + at, area.y, w[i], area.h),
                Rect::new(area.x + at + w[i], area.y, m.gap, area.h),
            ),
            Axis::Vertical => (
                Rect::new(area.x, area.y + at, area.w, w[i]),
                Rect::new(area.x, area.y + at + w[i], area.w, m.gap),
            ),
        };
        panes.push((tab, pane));
        at += w[i];
        if i + 1 < n {
            dividers.push(line);
            at += m.gap;
        }
    }
    Layout { panes, dividers, paired: true }
}

impl Split {
    fn sizes_or_even(&self) -> Vec<f64> {
        if self.sizes.len() == self.tabs.len() { self.sizes.clone() } else { even(self.tabs.len()) }
    }
}

/// Which divider the point is on, within `slop` pixels of it (the divider is
/// only `gap` wide, so give it a grabbable margin).
pub fn hit_test(layout: &Layout, p: Point, slop: i32) -> Option<usize> {
    layout.dividers.iter().position(|d| d.inflate(slop).contains(p))
}

/// A divider being held (PaneStage.dividerBegan / dividerMoved / dividerEnded,
/// PaneStage.swift:523-571). Two pages either side of it share their room
/// between them; the others stay as they are. Neither is narrower than
/// `narrowest`; near even it settles there (snap), staying until the pointer
/// goes `unsnap` past.
#[derive(Clone, Debug)]
pub struct Drag {
    index: usize,
    shares: Vec<f64>,
    room: f64,
    gap: f64,
    narrowest: f64,
    snap: f64,
    unsnap: f64,
    snapped: bool,
    start_left: f64,
    /// Set by the last move that settled on even (upstream's haptic tick).
    pub just_snapped: bool,
}

impl Drag {
    /// Begin on `split`'s divider `index` (after page `index`) in an area
    /// `length` long along the axis. None when no such divider.
    pub fn begin(split: &Split, index: usize, length: i32, m: &Metrics) -> Option<Drag> {
        let n = split.tabs.len();
        if index + 1 >= n {
            return None;
        }
        let room = room(length, n, m) as f64;
        // Of three or four, as drawn, so the pages don't jump; of two, as kept.
        let shares = if n > 2 && room > 0.0 {
            widths_f(room, &split.sizes_or_even(), m.narrowest as f64).iter().map(|w| w / room).collect()
        } else {
            split.sizes_or_even()
        };
        let pair = shares[index] + shares[index + 1];
        let snapped = (shares[index] - pair / 2.0).abs() < 0.0001;
        let start = Self::start_of(&shares, index, room, m.gap as f64);
        Some(Drag {
            index,
            start_left: shares[index] * room + start,
            shares,
            room,
            gap: m.gap as f64,
            narrowest: m.narrowest as f64,
            snap: m.snap as f64,
            unsnap: m.unsnap as f64,
            snapped,
            just_snapped: false,
        })
    }

    fn start_of(shares: &[f64], index: usize, room: f64, gap: f64) -> f64 {
        shares[..index].iter().sum::<f64>() * room + index as f64 * gap
    }

    /// The pointer at `pos` pixels from the area's start along the axis.
    /// True when the shares changed.
    pub fn to(&mut self, pos: i32) -> bool {
        let start = Self::start_of(&self.shares, self.index, self.room, self.gap);
        self.move_left(pos as f64 - self.gap / 2.0 - start)
    }

    /// The pointer `delta` pixels from where the divider was grabbed.
    pub fn by(&mut self, delta: i32) -> bool {
        let start = Self::start_of(&self.shares, self.index, self.room, self.gap);
        // The grabbed divider's left page began `start_left - start` wide.
        let grabbed = self.start_left - start;
        self.move_left(grabbed + delta as f64)
    }

    fn move_left(&mut self, wanted: f64) -> bool {
        self.just_snapped = false;
        if self.room <= 0.0 {
            return false;
        }
        let i = self.index;
        let both = (self.shares[i] + self.shares[i + 1]) * self.room;
        let mut left = wanted.max(self.narrowest).min(both - self.narrowest);
        let even = both / 2.0;
        if self.snapped {
            if (left - even).abs() > self.unsnap {
                self.snapped = false;
            } else {
                left = even;
            }
        } else if (left - even).abs() <= self.snap {
            self.snapped = true;
            self.just_snapped = true;
            left = even;
        }
        let first = left / self.room;
        let second = (both - left) / self.room;
        if first == self.shares[i] {
            return false;
        }
        self.shares[i] = first;
        self.shares[i + 1] = if self.shares.len() == 2 { 1.0 - first } else { second };
        true
    }

    /// Each page's share now; give them to `Split::set_sizes` when the
    /// divider is let go (and `layout` them meanwhile with `apply`).
    pub fn shares(&self) -> &[f64] {
        &self.shares
    }

    /// `split` with the live shares, unclamped, to `layout` while dragging.
    pub fn apply(&self, split: &Split) -> Split {
        Split { sizes: self.shares.clone(), ..split.clone() }
    }
}

/// New shares for divider `index` moved by `delta` pixels (one step, no
/// snapping state kept): what an arrow key or VoiceOver does.
pub fn resized(split: &Split, index: usize, delta: i32, length: i32, m: &Metrics) -> Option<Vec<f64>> {
    let mut d = Drag::begin(split, index, length, m)?;
    d.by(delta);
    Some(clamp(d.shares()))
}
