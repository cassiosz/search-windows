//! searchwin-split — Split View's model and geometry, no Win32.
//!
//! Ports upstream's `TabSplit` (Split.swift), the layout and divider maths of
//! `PaneStage` (PaneStage.swift) and `Session.Split` (Session.swift:51-58,
//! 172-199). Pixels are integers: panes and gaps always cover the area
//! exactly, with no rounding gap and no overlap.

mod geometry;
mod model;
mod session;

pub use geometry::{Drag, Layout, Metrics, Point, Rect, fits, hit_test, layout, resized};
pub use model::{Axis, MOST, Removed, Split, Splits, clamp, even, least};
pub use session::{from_session, to_session};

#[cfg(test)]
mod tests;
